"use client";

import { useSyncExternalStore } from "react";

// Hydration is the only transition: there is no changing browser value to subscribe to.
const subscribe = () => () => {};
const getSnapshot = () => true;
const getServerSnapshot = () => false;

/** Keep the server snapshot until hydration, then expose browser-only observations. */
export function useHydrated() {
  return useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
