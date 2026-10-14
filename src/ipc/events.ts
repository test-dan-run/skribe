import { listen as tauriListen } from "@tauri-apps/api/event";

import type { Events } from "./generated";

/**
 * Listens for an event Rust pushes to this window. Event names and payloads are checked
 * against the types generated from Rust. Returns a function that stops listening.
 */
export function listen<K extends keyof Events>(
  name: K,
  handler: (payload: Events[K]) => void,
): () => void {
  let stopped = false;
  let stop: (() => void) | null = null;
  void tauriListen<Events[K]>(name, (event) => {
    if (!stopped) {
      handler(event.payload);
    }
  }).then(
    (unlisten) => {
      // Registration is asynchronous, so the caller may have stopped before it finished.
      if (stopped) {
        unlisten();
      } else {
        stop = unlisten;
      }
    },
    () => {
      // Outside the app there is nothing to listen to. Commands report that state.
    },
  );
  return () => {
    stopped = true;
    stop?.();
  };
}
