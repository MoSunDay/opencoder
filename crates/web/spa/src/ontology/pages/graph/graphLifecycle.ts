import type { Graph } from "@antv/g6";

/** Graphin starts async renders without waiting before destroy (including StrictMode cleanup).
 * Serialize renders and let an in-flight render finish before releasing its G6 runtime.
 * Queued renders are cancelled on disposal; actual rendering errors still reject the caller.
 */
export function coordinateGraphLifecycle(graph: Pick<Graph, "render" | "destroy" | "destroyed">): void {
  const render = graph.render.bind(graph);
  const destroy = graph.destroy.bind(graph);
  let pending = Promise.resolve();
  let rendering = false;
  let disposing = false;

  graph.render = () => {
    const run = async () => {
      if (disposing || graph.destroyed) return;
      rendering = true;
      try { await render(); }
      finally {
        rendering = false;
        if (disposing && !graph.destroyed) destroy();
      }
    };
    pending = pending.then(run, run);
    return pending;
  };
  graph.destroy = () => {
    if (disposing || graph.destroyed) return;
    disposing = true;
    if (!rendering) destroy();
  };
}
