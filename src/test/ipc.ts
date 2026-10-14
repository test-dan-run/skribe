import type { Mock } from "vitest";

import type { CommandError, Commands, Events } from "../ipc";

type CommandName = keyof Commands;
type Outcome<K extends CommandName> =
  | { ok: true; value: Commands[K]["result"] }
  | { ok: false; error: CommandError };

/** What each command answers. A command missing here fails the test that calls it. */
export type Answers = {
  [K in CommandName]?: Outcome<K> | ((args: Commands[K]["args"]) => Outcome<K>);
};

/** Makes the mocked `call` answer by command name, whatever order commands arrive in. */
export function answer(callMock: Mock, answers: Answers): void {
  callMock.mockImplementation((name: CommandName, args?: unknown) => {
    const response = answers[name] as unknown;
    if (response === undefined) {
      throw new Error(`the test did not expect the command ${name}`);
    }
    return Promise.resolve(
      typeof response === "function" ? (response as (args: unknown) => unknown)(args) : response,
    );
  });
}

type Handlers = { [K in keyof Events]?: (payload: Events[K]) => void };

/** Captures the handlers the mocked `listen` is given, so a test can deliver events. */
export function captureEvents(listenMock: Mock) {
  const handlers: Handlers = {};
  const stopped: (keyof Events)[] = [];
  listenMock.mockImplementation((name: keyof Events, handler: (payload: never) => void) => {
    (handlers as Record<string, unknown>)[name] = handler;
    return () => {
      stopped.push(name);
    };
  });
  return {
    emit<K extends keyof Events>(name: K, payload: Events[K]) {
      const handler = handlers[name];
      if (handler === undefined) {
        throw new Error(`nothing is listening for ${name}`);
      }
      handler(payload);
    },
    stopped,
  };
}
