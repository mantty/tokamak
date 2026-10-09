// The tokamak Vite plugin exports this class from the entry Worker in tokamak builds and
// development, where the runtime and `tok dev` call `dispatch` to deliver events.
import { WorkerEntrypoint } from "cloudflare:workers";

import { dispatch } from "./registry.ts";

export class TokamakEvents extends WorkerEntrypoint {
  /** Runs the listeners of the event `name` and returns `{ reply, listened }`. */
  dispatch(name, event) {
    return dispatch(name, event, this.env, this.ctx);
  }
}
