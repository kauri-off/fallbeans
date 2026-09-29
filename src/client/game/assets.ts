import * as THREE from 'three';
import { MeshoptDecoder } from 'three/addons/libs/meshopt_decoder.module.js';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { MODEL_NAMES, type ModelName } from '../../sim/builder';

const models = new Map<ModelName, THREE.Object3D>();
/** Geometries owned by loaded models: never disposed with a map. */
export const sharedGeometries = new Set<THREE.BufferGeometry>();

export async function loadModels(onProgress?: (done: number, total: number) => void): Promise<void> {
  const loader = new GLTFLoader();
  loader.setMeshoptDecoder(MeshoptDecoder);
  let done = 0;
  await Promise.all(
    MODEL_NAMES.map(async (n) => {
      const gltf = await loader.loadAsync(`${import.meta.env.BASE_URL}models/${n}.glb`);
      gltf.scene.traverse((o) => {
        if (o instanceof THREE.Mesh) {
          o.castShadow = n !== 'cloud';
          o.receiveShadow = n !== 'cloud';
          sharedGeometries.add(o.geometry);
          const mats = Array.isArray(o.material) ? o.material : [o.material];
          for (const m of mats) {
            m.side = THREE.FrontSide;
            if (m instanceof THREE.MeshStandardMaterial) m.envMapIntensity = 0.8;
          }
        }
      });
      models.set(n, gltf.scene);
      onProgress?.(++done, MODEL_NAMES.length);
    }),
  );
}

function get(name: ModelName): THREE.Object3D {
  const m = models.get(name);
  if (!m) throw new Error(`model not loaded: ${name}`);
  return m;
}

export function clone(name: ModelName): THREE.Object3D {
  return get(name).clone(true);
}

/** Meshes of a model with their transform relative to the model root. */
export function meshParts(name: ModelName): { mesh: THREE.Mesh; local: THREE.Matrix4 }[] {
  const root = get(name);
  root.updateMatrixWorld(true);
  const inv = root.matrixWorld.clone().invert();
  const parts: { mesh: THREE.Mesh; local: THREE.Matrix4 }[] = [];
  root.traverse((o) => {
    if (o instanceof THREE.Mesh) parts.push({ mesh: o, local: inv.clone().multiply(o.matrixWorld) });
  });
  return parts;
}
