/** Keyboard, mouse (pointer lock) and gamepad, sampled once per simulation tick. */
export interface Sample {
  /** Camera-relative: x right, y forward, both −1…1. */
  moveX: number;
  moveY: number;
  jump: boolean;
  dive: boolean;
  grab: boolean;
  /** Scripted input may give a world-space direction instead (x, z), bypassing the camera. */
  world?: [number, number];
}

/** Input played by the debug probe or tests instead of the devices, until `until` (performance.now()). */
export interface ScriptedInput {
  moveX: number;
  moveY: number;
  world?: [number, number];
  /** Held buttons: a jump or dive fires once per press (see Input.press). */
  grab: boolean;
  until: number;
}

const KEYS = {
  up: ['KeyW', 'ArrowUp'],
  down: ['KeyS', 'ArrowDown'],
  left: ['KeyA', 'ArrowLeft'],
  right: ['KeyD', 'ArrowRight'],
  jump: ['Space'],
  dive: ['KeyE', 'ShiftLeft', 'ShiftRight', 'ControlLeft'],
  grab: ['KeyQ'],
};

export class Input {
  private readonly down = new Set<string>();
  private jumpEdge = false;
  private diveEdge = false;
  private mouseGrab = false;
  private padJump = false;
  private padDive = false;
  private dragging = false;
  /** Accumulated mouse / right-stick look since the last read (radians). */
  lookX = 0;
  lookY = 0;
  sensitivity = 1;
  invertY = false;
  enabled = true;
  onEmote: ((e: number) => void) | null = null;
  onEscape: (() => void) | null = null;
  /** Spectating: switch to the previous (−1) or next (+1) player. */
  onCycle: ((dir: number) => void) | null = null;
  /** Enter: the player wants to type a chat line. */
  onChat: (() => void) | null = null;
  spectating = false;
  /** Debug/tests: replaces keyboard, mouse and gamepad while active (works with the menu open). */
  script: ScriptedInput | null = null;
  private scriptJump = false;
  private scriptDive = false;

  /** Debug/tests: one jump or dive on the next sampled tick. */
  press(button: 'jump' | 'dive') {
    if (button === 'jump') this.scriptJump = true;
    else this.scriptDive = true;
  }

  constructor(private readonly canvas: HTMLCanvasElement) {
    window.addEventListener('keydown', (e) => this.key(e, true));
    window.addEventListener('keyup', (e) => this.key(e, false));
    window.addEventListener('blur', () => {
      this.down.clear();
      this.mouseGrab = false;
    });
    canvas.addEventListener('mousedown', (e) => {
      if (!this.locked) return;
      if (e.button === 0) this.diveEdge = true;
      if (e.button === 2) this.mouseGrab = true;
    });
    window.addEventListener('mouseup', (e) => {
      if (e.button === 2) this.mouseGrab = false;
    });
    canvas.addEventListener('contextmenu', (e) => e.preventDefault());
    // Without pointer lock (denied, or not yet clicked): drag with the right button to look.
    canvas.addEventListener('mousedown', (e) => {
      if (!this.locked && e.button === 2) this.dragging = true;
    });
    window.addEventListener('mouseup', (e) => {
      if (e.button === 2) this.dragging = false;
    });
    document.addEventListener('mousemove', (e) => {
      if (!this.locked && !this.dragging) return;
      this.lookX += e.movementX * 0.0022 * this.sensitivity;
      this.lookY += e.movementY * 0.0022 * this.sensitivity * (this.invertY ? -1 : 1);
    });
  }

  get locked() {
    return document.pointerLockElement === this.canvas;
  }

  async lock() {
    if (this.locked) return;
    try {
      // Raw mouse input where the browser supports it (Chromium).
      await (this.canvas.requestPointerLock as (o?: { unadjustedMovement?: boolean }) => Promise<void>)({
        unadjustedMovement: true,
      });
    } catch {
      try {
        await this.canvas.requestPointerLock();
      } catch {}
    }
  }

  unlock() {
    if (this.locked) document.exitPointerLock();
  }

  /** Lets go of every held key and button (the keyboard goes elsewhere: the chat line). */
  release() {
    this.down.clear();
    this.mouseGrab = false;
  }

  private key(e: KeyboardEvent, isDown: boolean) {
    const target = e.target as HTMLElement | null;
    if (target && (target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable)) return;
    if (isDown && e.code === 'Escape') this.onEscape?.();
    if (e.code === 'Tab') {
      e.preventDefault();
      return;
    }
    if (!this.enabled) return;
    if (isDown && !e.repeat && (e.code === 'Enter' || e.code === 'NumpadEnter')) {
      e.preventDefault();
      this.onChat?.();
      return;
    }
    if (this.spectating && isDown && !e.repeat) {
      if (e.code === 'ArrowLeft' || e.code === 'KeyQ' || e.code === 'KeyA') this.onCycle?.(-1);
      if (e.code === 'ArrowRight' || e.code === 'KeyE' || e.code === 'KeyD') this.onCycle?.(1);
    }
    if (isDown && !e.repeat) {
      if (KEYS.jump.includes(e.code)) this.jumpEdge = true;
      if (KEYS.dive.includes(e.code)) this.diveEdge = true;
      if (/^Digit[1-5]$/.test(e.code)) this.onEmote?.(Number(e.code.slice(5)));
    }
    if (e.code === 'Space' || e.code.startsWith('Arrow')) e.preventDefault();
    if (isDown) this.down.add(e.code);
    else this.down.delete(e.code);
  }

  private has(codes: string[]) {
    return codes.some((c) => this.down.has(c));
  }

  private pad(): Gamepad | null {
    for (const p of navigator.getGamepads?.() ?? []) if (p?.connected) return p;
    return null;
  }

  /** Reads movement and consumes jump/dive presses. Call once per simulation tick. */
  sample(dt: number): Sample {
    const sc = this.script && performance.now() < this.script.until ? this.script : null;
    if (this.script && !sc) this.script = null;
    if (sc || this.scriptJump || this.scriptDive) {
      const s: Sample = {
        moveX: sc?.moveX ?? 0,
        moveY: sc?.moveY ?? 0,
        jump: this.scriptJump,
        dive: this.scriptDive,
        grab: sc?.grab ?? false,
        ...(sc?.world ? { world: sc.world } : {}),
      };
      this.scriptJump = false;
      this.scriptDive = false;
      this.jumpEdge = false;
      this.diveEdge = false;
      return s;
    }
    let mx = (this.has(KEYS.right) ? 1 : 0) - (this.has(KEYS.left) ? 1 : 0);
    let my = (this.has(KEYS.up) ? 1 : 0) - (this.has(KEYS.down) ? 1 : 0);
    let grab = this.mouseGrab || this.has(KEYS.grab);
    const pad = this.pad();
    if (pad) {
      const dz = (v: number) => (Math.abs(v) < 0.15 ? 0 : v);
      const lx = dz(pad.axes[0] ?? 0);
      const ly = dz(pad.axes[1] ?? 0);
      if (lx || ly) {
        mx = lx;
        my = -ly;
      }
      const rx = dz(pad.axes[2] ?? 0);
      const ry = dz(pad.axes[3] ?? 0);
      this.lookX += rx * 3.2 * dt * this.sensitivity;
      this.lookY += ry * 2.2 * dt * this.sensitivity * (this.invertY ? -1 : 1);
      const b = (i: number) => !!pad.buttons[i]?.pressed;
      if (b(0) && !this.padJump) this.jumpEdge = true;
      if ((b(2) || b(1)) && !this.padDive) this.diveEdge = true;
      this.padJump = b(0);
      this.padDive = b(2) || b(1);
      grab ||= b(5) || b(7);
      if (b(12)) this.onEmote?.(1);
      if (b(14)) this.onEmote?.(2);
      if (b(15)) this.onEmote?.(3);
      if (b(13)) this.onEmote?.(4);
    }
    const l = Math.hypot(mx, my);
    if (l > 1) {
      mx /= l;
      my /= l;
    }
    const s: Sample = this.enabled
      ? { moveX: mx, moveY: my, jump: this.jumpEdge, dive: this.diveEdge, grab }
      : { moveX: 0, moveY: 0, jump: false, dive: false, grab: false };
    this.jumpEdge = false;
    this.diveEdge = false;
    return s;
  }
}
