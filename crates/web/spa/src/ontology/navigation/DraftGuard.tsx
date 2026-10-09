import { App } from "antd";
import { createContext, useCallback, useContext, useEffect, useId, useMemo, useRef, type ReactNode } from "react";

type Draft = { dirty: boolean; saving: boolean };
type Guard = {
  register: (id: string, value?: Draft) => void;
  blocked: () => boolean;
  run: (action: () => void, cancel?: () => void, local?: Draft) => void;
};
const Context = createContext<Guard | null>(null);

export function DraftGuardProvider({ children }: { children: ReactNode }) {
  const { modal, message } = App.useApp();
  const drafts = useRef(new Map<string, Draft>());
  const dialogOpen = useRef(false);
  const approved = useRef(false);
  const register = useCallback((id: string, value?: Draft) => {
    if (value) drafts.current.set(id, value); else drafts.current.delete(id);
  }, []);
  const blocked = useCallback(() => !approved.current && [...drafts.current.values()].some((item) => item.dirty || item.saving), []);
  const run = useCallback<Guard["run"]>((action, cancel, local) => {
    const values = [...drafts.current.values(), ...(local ? [local] : [])];
    if (values.some((item) => item.saving)) { message.info("内容正在保存，请稍候"); cancel?.(); return; }
    if (!values.some((item) => item.dirty)) { action(); return; }
    if (dialogOpen.current) return;
    dialogOpen.current = true;
    modal.confirm({ title: "有内容尚未保存", content: "离开后将放弃当前修改。", okText: "放弃并继续", cancelText: "继续编辑",
      onOk: () => { dialogOpen.current = false; approved.current = true; action(); setTimeout(() => { approved.current = false; }, 0); },
      onCancel: () => { dialogOpen.current = false; cancel?.(); } });
  }, [message, modal]);
  const value = useMemo(() => ({ register, blocked, run }), [register, blocked, run]);
  useEffect(() => {
    const beforeUnload = (event: BeforeUnloadEvent) => { if (blocked()) { event.preventDefault(); event.returnValue = ""; } };
    window.addEventListener("beforeunload", beforeUnload);
    return () => window.removeEventListener("beforeunload", beforeUnload);
  }, [blocked]);
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useDraftGuard(dirty = false, saving = false) {
  const guard = useContext(Context);
  const { modal, message } = App.useApp();
  const id = useId();
  useEffect(() => {
    guard?.register(id, { dirty, saving });
    return () => guard?.register(id);
  }, [guard, id, dirty, saving]);
  const runLocal = (action: () => void, cancel?: () => void) => {
    if (saving) { message.info("内容正在保存，请稍候"); cancel?.(); return; }
    if (dirty) modal.confirm({ title: "有内容尚未保存", content: "离开后将放弃当前修改。", okText: "放弃并继续", cancelText: "继续编辑", onOk: action, onCancel: cancel });
    else action();
  };
  const run = (action: () => void, cancel?: () => void) => guard ? guard.run(action, cancel, { dirty, saving }) : runLocal(action, cancel);
  return { run, runLocal, blocked: guard?.blocked ?? (() => dirty || saving) };
}
