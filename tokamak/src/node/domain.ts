import EventEmitter from "../events/events.ts";

export class Domain extends EventEmitter {
  add<T>(value: T): T { return value; }
  remove<T>(value: T): T { return value; }
  run<Args extends unknown[], R>(callback: (...args: Args) => R, ...args: Args): R { return callback(...args); }
  enter() {}
  exit() {}
}
export const active = undefined;
export function create() { return new Domain(); }
export const createDomain = create;
export default { Domain, active, create, createDomain };
