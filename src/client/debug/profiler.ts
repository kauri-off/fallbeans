import * as THREE from 'three';
import type { Game } from '../game/game';

/**
 * What costs the most performance, on this machine and this map:
 *   scene()  — what is drawn, by category (map, beans, clouds, sky…): draw calls, shadow draws, triangles;
 *   passes() — GPU time per render pass (scene, shadow maps, GTAO, bloom, SMAA…) and CPU time per frame section;
 *   ablate() — switches each feature off in turn and measures what that saves (GPU and CPU);
 *   run()    — all of it, plus the GPU and long main-thread tasks.
 * GPU times need EXT_disjoint_timer_query_webgl2 (desktop Chromium has it); without it the frame
 * time is used, which vsync caps, so savings below the refresh interval do not show.
 */

type Category = string;

interface Experiment {
  name: string;
  what: string;
  apply(): void;
  revert(): void;
}

const r2 = (v: number) => Math.round(v * 100) / 100;
const HIDDEN_LAYER = 31;

export function createProfiler(game: Game) {
  const rd = game.renderer;
  const longTasks: { at: number; ms: number }[] = [];
  try {
    new PerformanceObserver((list) => {
      for (const e of list.getEntries()) {
        longTasks.push({ at: Math.round(e.startTime), ms: Math.round(e.duration) });
        if (longTasks.length > 100) longTasks.shift();
      }
    }).observe({ type: 'longtask', buffered: true });
  } catch {}

  /** Resolves after n animation frames (or times out when the tab is hidden and frames stop). */
  const frames = (n: number) =>
    new Promise<boolean>((resolve) => {
      let k = 0;
      const t0 = performance.now();
      const timeout = setTimeout(() => resolve(false), n * 100 + 2000);
      const f = () => {
        if (++k >= n) {
          clearTimeout(timeout);
          resolve(performance.now() - t0 < n * 100);
        } else requestAnimationFrame(f);
      };
      requestAnimationFrame(f);
    });

  const categoryOf = (o: THREE.Object3D): Category => {
    const beans = new Set<THREE.Object3D>([...game.inspect().beans.values()].map((b) => b.root));
    const map: THREE.Object3D | undefined = game.arena?.builder.group;
    for (let x: THREE.Object3D | null = o; x; x = x.parent) {
      if (x === rd.sky) return 'sky';
      if (x === rd.motes) return 'motes';
      if (typeof x.userData.cat === 'string') return x.userData.cat;
      if (beans.has(x)) return 'beans';
      if (x === map) return 'map';
    }
    return 'other';
  };

  /** What gets drawn this frame, by category (frustum culling applied like the renderer does). */
  const scene = () => {
    const cam = rd.camera;
    cam.updateMatrixWorld();
    const frustum = new THREE.Frustum().setFromProjectionMatrix(
      new THREE.Matrix4().multiplyMatrices(cam.projectionMatrix, cam.matrixWorldInverse),
    );
    const shadowsOn = rd.renderer.shadowMap.enabled && rd.sun.castShadow;
    const cats = new Map<
      Category,
      { objects: number; draws: number; shadowDraws: number; triangles: number; shadowTriangles: number; culled: number }
    >();
    const heavy: { name: string; cat: string; triangles: number; draws: number; shadow: boolean; instances: number }[] = [];
    const materials = new Set<THREE.Material>();
    rd.scene.traverseVisible((o) => {
      if (!(o instanceof THREE.Mesh || o instanceof THREE.Points || o instanceof THREE.Line)) return;
      const cat = categoryOf(o);
      const c = cats.get(cat) ?? { objects: 0, draws: 0, shadowDraws: 0, triangles: 0, shadowTriangles: 0, culled: 0 };
      cats.set(cat, c);
      const geo = o.geometry as THREE.BufferGeometry;
      const instances = o instanceof THREE.InstancedMesh ? o.count : 1;
      const perDraw = o instanceof THREE.Mesh ? ((geo.index?.count ?? geo.attributes.position?.count ?? 0) / 3) * instances : 0;
      const mats = Array.isArray(o.material) ? o.material : [o.material];
      for (const m of mats) materials.add(m);
      const draws = Array.isArray(o.material) ? Math.max(1, geo.groups.length) : 1;
      const inView = !o.frustumCulled || frustum.intersectsObject(o);
      const shadow = shadowsOn && o instanceof THREE.Mesh && o.castShadow;
      c.objects++;
      if (inView) {
        c.draws += draws;
        c.triangles += perDraw;
      } else c.culled++;
      if (shadow) {
        c.shadowDraws += draws;
        c.shadowTriangles += perDraw;
      }
      heavy.push({
        name: pathName(o),
        cat,
        triangles: Math.round(perDraw),
        draws: (inView ? draws : 0) + (shadow ? draws : 0),
        shadow,
        instances,
      });
    });
    heavy.sort((a, b) => b.triangles * (b.shadow ? 2 : 1) - a.triangles * (a.shadow ? 2 : 1));
    return {
      categories: Object.fromEntries(
        [...cats]
          .sort((a, b) => b[1].triangles - a[1].triangles)
          .map(([k, v]) => [k, { ...v, triangles: Math.round(v.triangles), shadowTriangles: Math.round(v.shadowTriangles) }]),
      ),
      materials: materials.size,
      programs: rd.memory.programs,
      heaviest: heavy.slice(0, 15),
    };
  };

  /**
   * GPU per pass and CPU per frame section, over `n` frames. The whole frame is timed first on its
   * own: many small timer queries can also count GPU idle gaps (ANGLE/D3D11), so when the passes add
   * up to much more than the frame, read their shares rather than their milliseconds.
   */
  const passes = async (n = 120) => {
    const hadGpu = rd.gpuMs.length > 0;
    if (!rd.measureGpu('frame')) {
      game.prof.reset();
      const ok = await frames(n);
      return { gpu: null, frameGpu: null, inflated: false, cpu: game.prof.report(), throttled: !ok };
    }
    await frames(10);
    rd.resetGpu();
    await frames(Math.ceil(n / 2));
    const frameGpu = gpuReport()?.total ?? null;
    rd.measureGpu('passes');
    await frames(10);
    rd.resetGpu();
    game.prof.reset();
    const ok = await frames(n);
    const gpu = gpuReport();
    const cpu = game.prof.report();
    rd.measureGpu(hadGpu ? 'frame' : 'off');
    const inflated = !!gpu && frameGpu !== null && gpu.total > frameGpu * 1.5 + 0.2;
    return { gpu, frameGpu, inflated, cpu, throttled: !ok };
  };

  /** GPU report per rendered frame (the renderer may draw each frame several times while profiling). */
  const gpuReport = () => {
    const g = rd.gpuReport();
    const k = rd.debugRepeat;
    if (!g || k === 1) return g;
    const d = (v: number) => Math.round((v / k) * 1000) / 1000;
    return { ...g, total: d(g.total), labels: g.labels.map((l) => ({ ...l, ms: d(l.ms) })) };
  };

  /** One measurement: GPU and CPU ms per frame, over `n` frames. */
  const measure = async (n: number) => {
    rd.resetGpu();
    game.prof.reset();
    const t0 = performance.now();
    const ok = await frames(n);
    const frameMs = (performance.now() - t0) / n;
    const cpu = Object.values(game.prof.report().sections).reduce((s, x) => s + x.perFrame, 0);
    return { gpu: gpuReport()?.total ?? null, cpu, frameMs, ok };
  };

  const hideLayers = (pick: (o: THREE.Object3D) => boolean) => {
    const saved: [THREE.Object3D, number][] = [];
    rd.scene.traverse((o) => {
      if (!pick(o)) return;
      saved.push([o, o.layers.mask]);
      o.layers.set(HIDDEN_LAYER);
    });
    return () => {
      for (const [o, m] of saved) o.layers.mask = m;
    };
  };

  const experiments = (): Experiment[] => {
    const list: Experiment[] = [];
    let undo: () => void = () => {};
    list.push({
      name: 'shadows',
      what: 'sun shadow maps (casting and sampling)',
      apply: () => {
        rd.sun.castShadow = false;
      },
      revert: () => {
        rd.sun.castShadow = true;
      },
    });
    for (const { name, pass } of rd.passes) {
      if (name === 'scene' || name === 'output') continue;
      list.push({
        name,
        what: `post pass "${name}"`,
        apply: () => {
          pass.enabled = false;
        },
        revert: () => {
          pass.enabled = true;
        },
      });
    }
    list.push({
      name: 'msaa',
      what: 'multisampling of the scene target',
      apply: () => {
        rd.debugMsaa = 0;
        rd.resize();
      },
      revert: () => {
        rd.debugMsaa = null;
        rd.resize();
      },
    });
    list.push({
      name: 'resolution½',
      what: 'render at half resolution (fill rate)',
      apply: () => {
        rd.debugScale = 0.5;
        rd.resize();
      },
      revert: () => {
        rd.debugScale = 1;
        rd.resize();
      },
    });
    const cats = new Set<string>();
    rd.scene.traverse((o) => {
      if (o instanceof THREE.Mesh || o instanceof THREE.Points) cats.add(categoryOf(o));
    });
    for (const cat of cats)
      list.push({
        name: cat,
        what: `everything in "${cat}"`,
        apply: () => {
          undo = hideLayers((o) => categoryOf(o) === cat);
        },
        revert: () => undo(),
      });
    return list;
  };

  /**
   * Turns each feature off in turn; what it saves is what it costs. Every experiment is compared with
   * baselines measured right before and after it (drift cancels out), and savings smaller than the
   * baseline noise are flagged.
   */
  const ablate = async (n = 60) => {
    const hadGpu = rd.gpuMs.length > 0;
    const gpuOk = rd.measureGpu('frame');
    await frames(20);
    const first = await measure(n);
    let before = first;
    const noise: number[] = [];
    const rows: {
      name: string;
      what: string;
      gpu: number | null;
      cpu: number;
      frameMs: number;
      savedGpu: number | null;
      savedCpu: number;
      /** Smaller than the noise between baselines: treat as zero. */
      withinNoise: boolean;
    }[] = [];
    for (const e of experiments()) {
      e.apply();
      // New shader programs compile in the first frames after a change: let them.
      await frames(25);
      const m = await measure(n);
      e.revert();
      await frames(15);
      const after = await measure(Math.ceil(n / 2));
      const baseGpu = before.gpu === null || after.gpu === null ? null : (before.gpu + after.gpu) / 2;
      const baseCpu = (before.cpu + after.cpu) / 2;
      if (before.gpu !== null && after.gpu !== null) noise.push(Math.abs(before.gpu - after.gpu));
      const savedGpu = m.gpu === null || baseGpu === null ? null : baseGpu - m.gpu;
      const limit = Math.max(0.02, ...noise.slice(-4)) * 2;
      rows.push({
        name: e.name,
        what: e.what,
        gpu: m.gpu === null ? null : r2(m.gpu),
        cpu: r2(m.cpu),
        frameMs: r2(m.frameMs),
        savedGpu: savedGpu === null ? null : r2(savedGpu),
        savedCpu: r2(baseCpu - m.cpu),
        withinNoise: savedGpu === null ? Math.abs(baseCpu - m.cpu) < 0.05 : Math.abs(savedGpu) < limit,
      });
      before = after;
    }
    if (!hadGpu) rd.measureGpu('off');
    rows.sort((a, b) => (b.savedGpu ?? b.savedCpu) - (a.savedGpu ?? a.savedCpu));
    const gpu = first.gpu === null || before.gpu === null ? null : r2((first.gpu + before.gpu) / 2);
    const frameMs = r2((first.frameMs + before.frameMs) / 2);
    const warnings: string[] = [];
    if (!gpuOk) warnings.push('no GPU timer queries: savings come from frame time, which vsync caps');
    if (gpu !== null && gpu < frameMs * 0.15)
      warnings.push(
        `the GPU is busy only ${Math.round((gpu / frameMs) * 100)}% of the frame: at this load clock scaling hides most savings (try Ultra, a bigger window, or a weaker GPU)`,
      );
    return {
      gpuTimer: gpuOk,
      baseline: {
        gpu,
        cpu: r2((first.cpu + before.cpu) / 2),
        frameMs,
        /** Typical change between neighbouring baselines (noise level). */
        noise: noise.length ? r2(noise.reduce((a, b) => a + b, 0) / noise.length) : null,
      },
      warnings,
      throttled: !first.ok,
      rows,
    };
  };

  const gpuInfo = () => {
    const gl = rd.renderer.getContext();
    const ext = gl.getExtension('WEBGL_debug_renderer_info');
    return {
      renderer: ext ? String(gl.getParameter(ext.UNMASKED_RENDERER_WEBGL)) : String(gl.getParameter(gl.RENDERER)),
      vendor: ext ? String(gl.getParameter(ext.UNMASKED_VENDOR_WEBGL)) : String(gl.getParameter(gl.VENDOR)),
      maxTexture: gl.getParameter(gl.MAX_TEXTURE_SIZE) as number,
      timerQuery: !!gl.getExtension('EXT_disjoint_timer_query_webgl2'),
    };
  };

  return {
    scene,
    passes,
    ablate,
    gpuInfo,
    longTasks: () => longTasks.slice(),
    /** Everything: device, scene breakdown, per-pass GPU and per-section CPU, and the ablation table. */
    async run(opts: { frames?: number; ablate?: boolean; repeat?: number } = {}) {
      const n = opts.frames ?? 90;
      rd.debugRepeat = Math.max(1, Math.min(8, opts.repeat ?? 3));
      game.autoQuality = false;
      try {
        return await collect(n, opts.ablate !== false);
      } finally {
        rd.debugRepeat = 1;
        game.autoQuality = true;
      }
    },
  };

  async function collect(n: number, withAblation: boolean) {
    return {
      device: gpuInfo(),
      quality: rd.quality,
      size: [rd.canvas.width, rd.canvas.height],
      map: game.arena?.info.game ?? null,
      scene: scene(),
      passes: await passes(n),
      /** Each frame was rendered this many times while measuring (GPU ms are per render). */
      repeat: rd.debugRepeat,
      ablation: withAblation ? await ablate(Math.round(n * 0.7)) : null,
      longTasks: longTasks.slice(-20),
    };
  }
}

export type Profiler = ReturnType<typeof createProfiler>;

function pathName(o: THREE.Object3D): string {
  const parts: string[] = [];
  for (let x: THREE.Object3D | null = o; x && parts.length < 3; x = x.parent) parts.push(x.name || x.type);
  return parts.reverse().join('/');
}
