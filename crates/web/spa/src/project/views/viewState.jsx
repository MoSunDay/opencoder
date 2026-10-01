import { createContext, useCallback, useContext, useState } from 'react';
const Context = createContext(null);
export function ProjectViewState({ children }) {
  const [views, setViews] = useState({});
  const update = useCallback((key, initial, next) => setViews((current) => ({ ...current, [key]: typeof next === 'function' ? next(current[key] ?? initial) : next })), []);
  return <Context.Provider value={{ views, update }}>{children}</Context.Provider>;
}
export function useProjectView(key, initial) {
  const context = useContext(Context);
  const [local, setLocal] = useState(initial);
  return context ? [context.views[key] ?? initial, (next) => context.update(key, initial, next)] : [local, setLocal];
}
