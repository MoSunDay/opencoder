import { forwardRef, useImperativeHandle, useRef } from "react";
import { Vector3, type Camera } from "three";
import { nodeTitle } from "../canvas/presentation";
import { edgeCurve } from "./edges";
import { typeColor, type Point, type SceneData } from "./model";
import { placeLabel, type LabelBox } from "./labelLayout";

export type LabelsHandle = { update: (camera: Camera, width: number, height: number) => void };
type Props = { data: SceneData; centers: string[]; remaining: Record<string, number>; entityTypeNames: Record<string, string>;
  relationshipTypeNames: Record<string, string>; selectedNodeId?: string; selectedEdgeId?: string; disabled: boolean;
  onNodeClick: (id: string) => void; onEdgeClick: (id: string) => void; onExpand: (id: string) => void };

export default forwardRef<LabelsHandle, Props>(function Labels(props, ref) {
  const elements = useRef(new Map<string, HTMLElement>());
  const leaders = useRef(new Map<string, SVGLineElement>());
  useImperativeHandle(ref, () => ({ update(camera, width, height) {
    const occupied: LabelBox[] = [];
    const place = (id: string, point: Point, offset: number) => {
      const element = elements.current.get(id);
      if (!element) return;
      const projected = new Vector3().copy(point).project(camera);
      const visible = Number.isFinite(projected.x) && projected.z > -1 && projected.z < 1 && Math.abs(projected.x) < 1.2 && Math.abs(projected.y) < 1.2;
      element.style.visibility = visible ? "visible" : "hidden";
      const anchor = { x: (projected.x + 1) * width / 2, y: (1 - projected.y) * height / 2 };
      const relation = id.startsWith("e:");
      const w = element.offsetWidth || (relation ? 40 : 156), h = element.offsetHeight || (relation ? 26 : 52);
      const preferred = relation ? anchor : { x: anchor.x, y: anchor.y + offset + h / 2 };
      const box = placeLabel(preferred, w, h, occupied, { width, height });
      element.dataset.anchorX = String(anchor.x); element.dataset.anchorY = String(anchor.y);
      element.style.left = `${box.x + w / 2}px`;
      element.style.top = `${box.y + (relation ? h / 2 : 0)}px`;
      element.style.zIndex = String(Math.round((1 - projected.z) * 1000));
      if (visible) occupied.push(box);
      const leader = leaders.current.get(id);
      if (leader) {
        const x = Math.max(box.x, Math.min(box.x + w, anchor.x)), y = Math.max(box.y, Math.min(box.y + h, anchor.y));
        leader.style.visibility = visible && Math.hypot(x - anchor.x, y - anchor.y) > 4 ? "visible" : "hidden";
        leader.setAttribute("x1", String(anchor.x)); leader.setAttribute("y1", String(anchor.y));
        leader.setAttribute("x2", String(x)); leader.setAttribute("y2", String(y));
      }
    };
    [...props.data.nodes].sort((a, b) => Number(props.centers.includes(b.id) || b.id === props.selectedNodeId)
      - Number(props.centers.includes(a.id) || a.id === props.selectedNodeId)).forEach((node) => place(`n:${node.id}`, node, 14));
    const nodes = new Map(props.data.nodes.map((node) => [node.id, node]));
    props.data.links.forEach((link) => {
      const start = typeof link.source === "string" ? nodes.get(link.source) : link.source;
      const end = typeof link.target === "string" ? nodes.get(link.target) : link.target;
      if (start && end) place(`e:${link.id}`, edgeCurve(start, end, link).getPoint(0.5), 0);
    });
  } }), [props.data, props.centers, props.selectedNodeId]);
  const register = (key: string) => (element: HTMLElement | null) => { if (element) elements.current.set(key, element); else elements.current.delete(key); };
  return <div className={`graph-3d-labels${props.data.nodes.length > 12 ? " compact" : ""}`}>
    <svg className="graph-3d-leaders" aria-hidden="true">{[...props.data.nodes.map((node) => `n:${node.id}`), ...props.data.links.map((link) => `e:${link.id}`)].map((id) => <line key={id}
      ref={(element) => { if (element) leaders.current.set(id, element); else leaders.current.delete(id); }} />)}</svg>
    {props.data.nodes.map((node) => {
      const title = nodeTitle(node.name);
      return <div key={node.id} ref={register(`n:${node.id}`)} data-node-id={node.id}
        className={`graph-3d-node${props.selectedNodeId === node.id ? " selected" : ""}${props.centers.includes(node.id) ? " center" : ""}`}
        style={{ borderColor: typeColor(node.typeId) }}>
        <button className="graph-3d-name" title={node.name} disabled={props.disabled} onClick={() => props.onNodeClick(node.id)}>
          <span className="graph-node-title">{title.title}</span>{title.detail ? <span className="graph-3d-path">{title.detail}</span> : null}
        </button>
        <div className="graph-3d-type"><span>{props.entityTypeNames[node.typeId] ?? "实体"}</span>
          {props.remaining[node.id] > 0 ? <button disabled={props.disabled} onClick={() => props.onExpand(node.id)} aria-label={`展开 ${node.name} 的相邻实体`}>+{props.remaining[node.id]}</button> : null}
        </div>
      </div>;
    })}
    {props.data.links.map((link) => <button key={link.id} ref={register(`e:${link.id}`)} data-edge-id={link.id} disabled={props.disabled}
      className={`graph-3d-relation${props.selectedEdgeId === link.id ? " selected" : ""}`}
      onClick={() => props.onEdgeClick(link.id)}>{props.relationshipTypeNames[link.typeId] ?? "关系"}</button>)}
  </div>;
});
