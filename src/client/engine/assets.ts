import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';

export const ASSET_NAMES = ['bean', 'crown', 'hub', 'arm', 'hammer', 'hex', 'door', 'finish', 'bumper', 'cloud'] as const;
export type AssetName = (typeof ASSET_NAMES)[number];

const assets = new Map<AssetName, THREE.Object3D>();
export const sharedGeometries = new Set<THREE.BufferGeometry>();

export async function loadAssets(onProgress?: (done: number, total: number) => void): Promise<void> {
  const loader = new GLTFLoader();
  let done = 0;
  await Promise.all(
    ASSET_NAMES.map(async (n) => {
      const gltf = await loader.loadAsync(`${import.meta.env.BASE_URL}assets/${n}.glb`);
      gltf.scene.traverse((o) => {
        if (o instanceof THREE.Mesh) {
          o.castShadow = true;
          o.receiveShadow = true;
          sharedGeometries.add(o.geometry);
          if (o.material instanceof THREE.Material) o.material.side = THREE.FrontSide;
        }
      });
      assets.set(n, gltf.scene);
      onProgress?.(++done, ASSET_NAMES.length);
    }),
  );
}

function get(name: AssetName): THREE.Object3D {
  const a = assets.get(name);
  if (!a) throw new Error(`asset not loaded: ${name}`);
  return a;
}

export function clone(name: AssetName): THREE.Object3D {
  return get(name).clone(true);
}

export function meshParts(name: AssetName): THREE.Mesh[] {
  const parts: THREE.Mesh[] = [];
  get(name).traverse((o) => {
    if (o instanceof THREE.Mesh) parts.push(o);
  });
  return parts;
}
