import { Vector3, type PerspectiveCamera } from "three";
import type { Point, SceneNode } from "./model";

export type CameraGraph = { camera: () => unknown; controls: () => unknown; cameraPosition: (position: Point, target: Point) => unknown };
export type Orbit = { target: Vector3; addEventListener: (name: "change", fn: () => void) => void; removeEventListener: (name: "change", fn: () => void) => void };

function initialDirection(nodes: SceneNode[]) {
  const candidates = [[0.3, 0.2, 1], [1, 0.4, 0.2], [-0.6, 0.5, 1], [-1, 0.3, 0.4], [0.2, 1, 0.4]];
  const sample = nodes.filter((_, index) => index % Math.max(1, Math.ceil(nodes.length / 80)) === 0);
  const score = (direction: Vector3) => {
    let minimum = Infinity;
    for (let i = 0; i < sample.length; i++) for (let j = i + 1; j < sample.length; j++) {
      const delta = new Vector3().copy(sample[i]).sub(sample[j]);
      minimum = Math.min(minimum, delta.lengthSq() - delta.dot(direction) ** 2);
    }
    return minimum;
  };
  return candidates.map(([x, y, z]) => new Vector3(x, y, z).normalize()).map((direction) => ({ direction, score: score(direction) }))
    .sort((a, b) => b.score - a.score)[0].direction;
}

export function focusCamera(graph: CameraGraph, point: Point) {
  const camera = graph.camera() as PerspectiveCamera;
  const controls = graph.controls() as Orbit;
  const offset = camera.position.clone().sub(controls.target);
  if (offset.lengthSq() < 1) offset.set(0, 0, 600);
  graph.cameraPosition(new Vector3().copy(point).add(offset), point);
}

export function zoomCamera(graph: CameraGraph, factor: number) {
  const camera = graph.camera() as PerspectiveCamera;
  const target = (graph.controls() as Orbit).target;
  const offset = camera.position.clone().sub(target);
  const distance = Math.min(50000, Math.max(40, offset.length() / factor));
  if (offset.lengthSq() < 1) offset.set(0, 0, 1);
  graph.cameraPosition(offset.normalize().multiplyScalar(distance).add(target), target);
}

export function frameCamera(graph: CameraGraph, nodes: SceneNode[], center?: SceneNode, reset = false) {
  if (!nodes.length) return;
  const camera = graph.camera() as PerspectiveCamera;
  const target = center ? new Vector3().copy(center) : new Vector3(
    (Math.min(...nodes.map((n) => n.x)) + Math.max(...nodes.map((n) => n.x))) / 2,
    (Math.min(...nodes.map((n) => n.y)) + Math.max(...nodes.map((n) => n.y))) / 2,
    (Math.min(...nodes.map((n) => n.z)) + Math.max(...nodes.map((n) => n.z))) / 2);
  const radius = Math.max(100, ...nodes.map((node) => target.distanceTo(node))) + 100;
  const halfFov = Math.atan(Math.tan(camera.fov * Math.PI / 360) * Math.min(1, camera.aspect));
  const distance = radius / Math.sin(halfFov);
  const direction = reset ? initialDirection(nodes) : camera.position.clone().sub((graph.controls() as Orbit).target);
  if (direction.lengthSq() < 1) direction.set(0.3, 0.2, 1);
  camera.far = Math.max(100000, distance * 4); camera.updateProjectionMatrix();
  graph.cameraPosition(direction.normalize().multiplyScalar(distance).add(target), target);
}
