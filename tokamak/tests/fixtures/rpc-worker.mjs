import { WorkerEntrypoint } from 'cloudflare:workers';
import { rpcResponse } from './endpoint.mjs';

class Native extends WorkerEntrypoint {
  echo(value) { return value; }
  async stage() {
    return JSON.parse(new TextDecoder().decode(await this.ctx[Symbol.for('tokamak.native')].call('tokamak', 'lifecycleStage', new Uint8Array())));
  }
  fail() { throw Object.assign(new Error('Denied'), { code: 'DENIED' }); }
  invalid() { return { callback() {} }; }
  bytes() {
    return new ReadableStream({
      type: 'bytes',
      start(controller) { controller.enqueue(new Uint8Array([1, 2, 3])); controller.close(); },
    });
  }
  ignore() { return 'finished'; }
}

export class TokamakEvents extends WorkerEntrypoint {
  dispatch() { return { reply: null, listened: [] }; }
}

export default {
  fetch(request, env, ctx) {
    return rpcResponse(request, scoped => new Native(scoped, env),
      ['echo', 'stage', 'fail', 'invalid', 'bytes', 'ignore'], ctx);
  },
};
