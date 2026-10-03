import { createContext, useContext } from "react";
import type { Environment } from "./types";
export type EnvContextValue = { env: string; environments: Environment[]; setEnv: (env: string) => void; refreshEnvironments: () => Promise<void>; canManage: boolean };
export const EnvContext = createContext<EnvContextValue>({ env: "debug", environments: [], setEnv: () => undefined, refreshEnvironments: async () => undefined, canManage: false });
export function useEnv() { return useContext(EnvContext); }
