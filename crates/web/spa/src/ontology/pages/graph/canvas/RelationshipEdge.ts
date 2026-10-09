import { ExtensionCategory, Polyline, register, type PolylineStyleProps } from "@antv/g6";

/** Use Dagre's reserved label position, including parallel and reverse relationships. */
class RelationshipEdge extends Polyline {
  protected getLabelStyle(attributes: Required<PolylineStyleProps>) {
    const style = super.getLabelStyle(attributes);
    const points = attributes.controlPoints;
    if (!style || !points?.length) return style;
    const point = points[Math.floor(points.length / 2)];
    return { ...style, x: point[0], y: point[1], transform: [], textAlign: "center" as const, textBaseline: "middle" as const };
  }

  protected getLoopPath(attributes: Required<PolylineStyleProps>) {
    return attributes.controlPoints?.length ? this.getKeyPath(attributes) : super.getLoopPath(attributes);
  }
}

export const RELATIONSHIP_EDGE = "complete-relationship";
register(ExtensionCategory.EDGE, RELATIONSHIP_EDGE, RelationshipEdge);
