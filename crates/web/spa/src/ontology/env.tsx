import { createContext, useContext } from "react";
// Each Server initializes this default namespace; the Server connection selects the environment.
export const SERVER_ENV = "debug";
export type EnvContextValue = { env: string; canManage: boolean };
export const EnvContext = createContext<EnvContextValue>({ env: SERVER_ENV, canManage: false });
export function useEnv() { return useContext(EnvContext); }
