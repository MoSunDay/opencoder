import { ConeGeometry, CubicBezierCurve3, Group, Mesh, MeshBasicMaterial, QuadraticBezierCurve3, TubeGeometry, Vector3 } from "three";
import type { Point, SceneLink } from "./model";

export function edgeCurve(start: Point, end: Point, link: SceneLink) {
  const a = new Vector3().copy(start), b = new Vector3().copy(end);
  if (a.distanceToSquared(b) < 0.01) {
    const radius = 120 * link.curvature;
    const side = new Vector3(Math.cos(link.rotation), Math.sin(link.rotation), 0).multiplyScalar(radius);
    return new CubicBezierCurve3(a, a.clone().add(side).add(new Vector3(0, 0, radius)), a.clone().sub(side).add(new Vector3(0, 0, radius)), b);
  }
  const delta = b.clone().sub(a);
  const axis = delta.clone().normalize();
  const sourceId = typeof link.source === "string" ? link.source : link.source.id;
  const targetId = typeof link.target === "string" ? link.target : link.target.id;
  if (sourceId > targetId) axis.negate();
  const perpendicular = new Vector3().crossVectors(axis, Math.abs(axis.y) < 0.9 ? new Vector3(0, 1, 0) : new Vector3(1, 0, 0));
  perpendicular.normalize().applyAxisAngle(axis, link.rotation).multiplyScalar(delta.length() * link.curvature);
  return new QuadraticBezierCurve3(a, a.clone().add(b).multiplyScalar(0.5).add(perpendicular), b);
}

export function createEdge(color: string) {
  const group = new Group();
  const material = new MeshBasicMaterial({ color });
  group.add(new Mesh(undefined, material), new Mesh(new ConeGeometry(3, 9, 8), material));
  return group;
}

export function updateEdge(group: Group, start: Point, end: Point, link: SceneLink) {
  const signature = [start.x, start.y, start.z, end.x, end.y, end.z, link.curvature, link.rotation].join();
  if (group.userData.signature === signature) return;
  group.userData.signature = signature;
  const curve = edgeCurve(start, end, link);
  const line = group.children[0] as Mesh;
  line.geometry.dispose(); line.geometry = new TubeGeometry(curve, 24, 0.7, 4, false);
  const arrow = group.children[1];
  arrow.position.copy(curve.getPoint(0.78));
  arrow.quaternion.setFromUnitVectors(new Vector3(0, 1, 0), curve.getTangent(0.78).normalize());
}

export function disposeEdge(group: Group) {
  const materials = new Set<MeshBasicMaterial>();
  for (const item of group.children as Mesh[]) { item.geometry.dispose(); materials.add(item.material as MeshBasicMaterial); }
  materials.forEach((material) => material.dispose());
}
