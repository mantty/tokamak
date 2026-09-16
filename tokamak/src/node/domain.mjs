import EventEmitter from "../events/events.mjs";

export class Domain extends EventEmitter {
  add(value) { return value; }
  remove(value) { return value; }
  run(callback, ...args) { return callback(...args); }
  enter() {}
  exit() {}
}
export const active = undefined;
export function create() { return new Domain(); }
export const createDomain = create;
export default { Domain, active, create, createDomain };
