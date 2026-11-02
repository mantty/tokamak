import { WorkerEntrypoint } from 'cloudflare:workers';
import type { NativeConnection } from '../../../plugins/core/src/context.ts';
import { rpcResponse } from './endpoint.ts';

class Native extends WorkerEntrypoint {
  echo(value: unknown) { return value; }
  async stage() {
    const native: NativeConnection = Reflect.get(this.ctx, Symbol.for('tokamak.native'));
    return JSON.parse(new TextDecoder().decode(await native.call('tokamak', 'lifecycleStage', new Uint8Array())));
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
  dispatch() { return { reply: new Uint8Array(), listened: [] }; }
}

export default {
  fetch(request: Request, env: Cloudflare.Env, ctx: ExecutionContext) {
    return rpcResponse(request, scoped => new Native(scoped, env),
      ['echo', 'stage', 'fail', 'invalid', 'bytes', 'ignore'], ctx);
  },
};
