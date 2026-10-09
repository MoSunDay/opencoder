import { useRef, useState } from "react";

export type DetailTarget = { kind: "entity" | "relationship"; id: string };
export type DetailView = { tab: string; scroll: number };
export function useDetailNavigation() {
  const [history, setHistory] = useState<DetailTarget[]>([]);
  const views = useRef(new Map<string, DetailView>());
  const current = history.at(-1);
  const open = (target: DetailTarget) => setHistory((previous) => {
    const last = previous.at(-1);
    return last?.id === target.id && last.kind === target.kind ? previous : [...previous.slice(-19), target];
  });
  return { current, canBack: history.length > 1, view: current ? views.current.get(`${current.kind}/${current.id}`) : undefined,
    openEntity: (id: string) => open({ kind: "entity", id }), openRelationship: (id: string) => open({ kind: "relationship", id }),
    back: () => setHistory((previous) => previous.slice(0, -1)), close: () => { setHistory([]); views.current.clear(); },
    remember: (view: DetailView) => { if (current) views.current.set(`${current.kind}/${current.id}`, view); } };
}
