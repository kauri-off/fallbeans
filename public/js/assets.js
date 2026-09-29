import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';

const NAMES = ['bean', 'crown', 'hub', 'arm', 'hammer', 'hex', 'door', 'finish', 'bumper', 'cloud'];
export const assets = {};
export const sharedGeometries = new Set();

export async function loadAssets() {
  const loader = new GLTFLoader();
  await Promise.all(NAMES.map(async (n) => {
    const gltf = await loader.loadAsync(`./assets/${n}.glb`);
    const root = gltf.scene;
    root.traverse((o) => {
      if (o.isMesh) {
        o.castShadow = true;
        o.receiveShadow = true;
        sharedGeometries.add(o.geometry);
        if (o.material) o.material.side = THREE.FrontSide;
      }
    });
    assets[n] = root;
  }));
}

export function clone(name) {
  return assets[name].clone(true);
}

export function meshParts(name) {
  const parts = [];
  assets[name].traverse((o) => { if (o.isMesh) parts.push(o); });
  return parts;
}
