import assert from 'node:assert/strict';
import fs from 'node:fs';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import test from 'node:test';

async function createRuntime(t, workers) {
  const require = createRequire(import.meta.resolve('@cloudflare/vite-plugin'));
  const { Miniflare, convertV4MiniflareOptions } = await import(pathToFileURL(require.resolve('miniflare')).href);
  const runtime = new Miniflare(convertV4MiniflareOptions({ workers }));
  t.after(() => runtime.dispose());
  return runtime;
}

function rpcModules() {
  return ['binding', 'client', 'endpoint', 'channel', 'frames', 'values', 'streams'].map(name => ({
    type: 'ESModule', path: `/rpc/${name}.mjs`,
    contents: fs.readFileSync(new URL(`../../plugins/core/src/rpc/${name}.mjs`, import.meta.url), 'utf8'),
  }));
}

async function service(t, caller) {
  const modules = rpcModules();
  const methods = ['echo', 'fail', 'invalid', 'count', 'ignore', 'consume', 'consumed', 'produce', 'producerCancelled', 'nested'];
  return createRuntime(t, [{
    name: 'main', compatibilityDate: '2025-01-01', modulesRoot: '/',
    serviceBindings: { NATIVE: 'native' },
    modules: [{
      type: 'ESModule', path: '/main.mjs',
      contents: `import { createBinding, createRpcPort } from './rpc/binding.mjs';
        import { WorkerEntrypoint, RpcTarget } from 'cloudflare:workers';
        export default class extends WorkerEntrypoint {
          __tokamakRpc(peer) {
            return createRpcPort(peer, () => ({ label(value) { return 'main:' + value; } }), ['label'], this.ctx);
          }
          async fetch(request) {
          const env = this.env, ctx = this.ctx;
          const native = createBinding(env.NATIVE, ${JSON.stringify(methods)});
          ${caller}
        } };`,
    }, ...modules],
  }, {
    name: 'native', compatibilityDate: '2025-01-01', modulesRoot: '/', serviceBindings: { MAIN: 'main' },
    modules: [{
      type: 'ESModule', path: '/native.mjs',
      contents: `import { createBinding, createRpcPort } from './rpc/binding.mjs';
        import { WorkerEntrypoint, RpcTarget } from 'cloudflare:workers';
        let invocations = 0, consumed, producerCancelled = false;
        export default class extends WorkerEntrypoint {
          fetch(request) {
            if (request.headers.get('upgrade') !== 'websocket') return new Response(request.body);
            const [client, server] = Object.values(new WebSocketPair());
            server.accept();
            return new Response(null, { status: 101, webSocket: client });
          }
          __tokamakRpc(peer) {
          return createRpcPort(peer, scope => ({
            echo(value) { invocations++; return value; },
            count() { return invocations; },
            fail(kind) {
              const error = new Error('Denied', { cause: { permission: 'location' } });
              error.code = 'DENIED';
              if (kind === 'stream') error.details = new ReadableStream({
                type: 'bytes', start(c) { c.enqueue(new Uint8Array([4, 5])); c.close(); }
              });
              if (kind === 'invalid') error.callback = () => {};
              throw error;
            },
            invalid(kind) {
              if (kind === 'callback') return () => {};
              if (kind === 'target') return new (class extends RpcTarget { run() {} })();
              if (kind === 'instance') return new (class { constructor() { this.value = 1; } })();
              if (kind === 'promise') return { nested: Promise.resolve(1) };
              if (kind === 'response') return new Response('unsupported');
              return new ReadableStream({ start(c) { c.enqueue({ value: 1 }); c.close(); } });
            },
            ignore() { return 'done'; },
            consume(input) {
              scope.waitUntil(new Response(input).arrayBuffer().then(buffer => { consumed = [...new Uint8Array(buffer)]; }));
              return 'queued';
            },
            consumed() { return consumed; },
            produce() { return new ReadableStream({
              type: 'bytes', async cancel() { await new Promise(r => setTimeout(r, 20)); producerCancelled = true; }
            }); },
            producerCancelled() { return producerCancelled; },
            nested: value => createBinding(this.env.MAIN, ['label']).label(value),
          }), ${JSON.stringify(methods)}, this.ctx);
        } };`,
    }, ...modules],
  }]);
}

for (const [name, handler] of Object.entries({
  class: `class Main extends WorkerEntrypoint {
    scheduled(event) { this.ctx.waitUntil(later([event.cron, this.env.LABEL])); }
    queue(batch) { this.ctx.waitUntil(later([batch.queue, batch.messages.map(message => message.body)])); batch.ackAll(); }
    fetch() { return Response.json(records); }
  }`,
  object: `const Main = {
    scheduled(event, env, ctx) { ctx.waitUntil(later([event.cron, env.LABEL])); },
    queue(batch, env, ctx) { ctx.waitUntil(later([batch.queue, batch.messages.map(message => message.body)])); batch.ackAll(); },
    fetch() { return Response.json(records); },
  };`,
})) {
  test(`development ${name} handlers receive scheduled and queue events, bindings, and invocation context`, { timeout: 10_000 }, async t => {
    const runtime = await createRuntime(t, [{
      name: 'handlers', compatibilityDate: '2025-01-01', modulesRoot: '/', bindings: { LABEL: 'configured' },
      modules: [{
        type: 'ESModule', path: '/main.mjs',
        contents: `import { WorkerEntrypoint } from 'cloudflare:workers';
          import { developmentEntrypoint } from './rpc/binding.mjs';
          const records = [];
          function later(value) { return new Promise(resolve => setTimeout(resolve, 5)).then(() => records.push(value)); }
          ${handler}
          export default developmentEntrypoint(Main, [], {});`,
      }, ...rpcModules()],
    }]);
    const worker = await runtime.getWorker('handlers');
    await worker.scheduled({ cron: '* * * * *' });
    const result = await worker.queue('items', [{ id: '1', timestamp: new Date(), body: { value: 1 }, attempts: 1 }]);
    assert.equal(result.ackAll, true);
    assert.deepEqual(await (await worker.fetch('https://handlers.example')).json(), [
      ['* * * * *', 'configured'], ['items', [{ value: 1 }]],
    ]);
  });
}

test('development bindings preserve the RPC value graph and error fields independently of compatibility dates', { timeout: 10_000 }, async t => {
  const runtime = await service(t, `
    const buffer = new Uint8Array([1, 2, 3, 4]).buffer;
    const value = {
      integer: 17n, date: new Date(123), regexp: /word/gi, set: new Set([2]),
      buffer, view: new Uint8Array(buffer, 1, 2), data: new DataView(buffer, 2),
      error: Object.assign(new Error('value', { cause: 'cause' }), { code: 'VALUE' }),
      exception: new DOMException('missing', 'NotFoundError'),
      nan: NaN, zero: -0, infinity: Infinity,
    };
    value.self = value;
    value.map = new Map([[value, value.view]]);
    const result = await native.echo(value);
    const nested = await native.nested('returned');
    let error;
    try { await native.fail('stream'); } catch (failure) { error = failure; }
    return Response.json({
      cycles: result.self === result && result.map.get(result) === result.view,
      views: result.buffer === result.view.buffer && result.buffer === result.data.buffer,
      copied: result !== value && result.buffer !== buffer,
      bytes: [...result.view], offset: result.data.byteOffset,
      integer: String(result.integer), date: result.date.getTime(), regexp: String(result.regexp),
      set: [...result.set], nan: Number.isNaN(result.nan), zero: Object.is(result.zero, -0), infinity: result.infinity === Infinity,
      valueError: [result.error.message, result.error.cause, result.error.code],
      exception: [result.exception.name, result.exception.message],
      error: [error.message, error.code, error.cause.permission],
      details: [...new Uint8Array(await new Response(error.details).arrayBuffer())], nested,
    });
  `);
  assert.deepEqual(await (await runtime.dispatchFetch('https://main.example')).json(), {
    cycles: true, views: true, copied: true, bytes: [2, 3], offset: 2,
    integer: '17', date: 123, regexp: '/word/gi', set: [2], nan: true, zero: true, infinity: true,
    valueError: ['value', 'cause', 'VALUE'], exception: ['NotFoundError', 'missing'],
    error: ['Denied', 'DENIED', 'location'], details: [4, 5], nested: 'main:returned',
  });
});

test('development bindings reject unsupported arguments before invoking user code and reject unsupported results', { timeout: 10_000 }, async t => {
  const runtime = await service(t, `
    const failures = [];
    const invalid = [
      () => {}, new (class extends RpcTarget { run() {} })(),
      new (class { constructor() { this.value = 1; } })(),
      { nested: Promise.resolve(1) }, new Response('unsupported'), Symbol('value'),
      Object.assign(new Error('invalid'), { callback() {} }),
    ];
    for (const value of invalid) {
      try { await native.echo(value); failures.push('accepted'); }
      catch (error) { failures.push(error.name); }
    }
    const invocations = await native.count();
    for (const kind of ['callback', 'target', 'instance', 'promise', 'response']) {
      try { await native.invalid(kind); failures.push('accepted'); }
      catch (error) { failures.push(error.name); }
    }
    try { await native.fail('invalid'); failures.push('accepted'); }
    catch (error) { failures.push(error.name); }
    return Response.json({ failures, invocations, dup: typeof native.dup, event: typeof native.event });
  `);
  assert.deepEqual(await (await runtime.dispatchFetch('https://main.example')).json(), {
    failures: Array(13).fill('DataCloneError'), invocations: 0, dup: 'undefined', event: 'undefined',
  });
});

test('development calls snapshot arguments and transfer stream ownership before returning to the caller', { timeout: 10_000 }, async t => {
  const runtime = await service(t, `
    const source = new ReadableStream({ type: 'bytes' });
    const input = { value: 1, bytes: new Uint8Array([2]), source };
    const pending = native.echo(input);
    const locked = source.locked;
    input.value = 3;
    input.bytes[0] = 4;
    const result = await pending;
    await result.source.cancel();
    const unavailable = createBinding({
      __tokamakRpc() { return Promise.reject(Object.assign(new Error('Unavailable'), { code: 'OFFLINE' })); }
    }, ['echo']);
    let error;
    try { await unavailable.echo('value'); } catch (failure) { error = failure; }
    return Response.json({ value: result.value, bytes: [...result.bytes], locked, error: [error.message, error.code] });
  `);
  assert.deepEqual(await (await runtime.dispatchFetch('https://main.example')).json(), {
    value: 1, bytes: [2], locked: true, error: ['Unavailable', 'OFFLINE'],
  });
});

test('development streams stay lazy, bound reads and preserve cancellation reasons across both workers', { timeout: 10_000 }, async t => {
  const runtime = await service(t, `
    let pulls = 0;
    const cancelled = Promise.withResolvers();
    const input = new ReadableStream({
      type: 'bytes',
      pull(c) { pulls++; c.enqueue(new Uint8Array(150000).fill(7)); },
      cancel(reason) { cancelled.resolve(reason); },
    });
    const output = await native.echo(input);
    const before = pulls;
    const reader = output.getReader();
    const first = await reader.read();
    const after = pulls;
    await reader.cancel(Object.assign(new Error('finished'), { code: 'CANCELLED' }));
    const reason = await cancelled.promise;
    let invalid;
    try { await (await native.invalid('stream')).getReader().read(); }
    catch (error) { invalid = error.name; }
    return Response.json({ before, after, bounded: first.value.byteLength > 0 && first.value.byteLength <= 65536, byte: first.value[0],
      locked: input.locked, reason: [reason.message, reason.code], invalid });
  `);
  assert.deepEqual(await (await runtime.dispatchFetch('https://main.example')).json(), {
    before: 0, after: 1, bounded: true, byte: 7, locked: true,
    reason: ['finished', 'CANCELLED'], invalid: 'DataCloneError',
  });
});

test('development calls cancel unused inputs and retain inputs needed by registered work', { timeout: 10_000 }, async t => {
  const runtime = await service(t, `
    const cancelled = Promise.withResolvers();
    const ignored = await native.ignore(new ReadableStream({ type: 'bytes', cancel: cancelled.resolve }));
    const reason = await cancelled.promise;
    let producer;
    const input = new ReadableStream({ type: 'bytes', start(c) { producer = c; } });
    const result = await native.consume(input);
    producer.enqueue(new Uint8Array([8, 9]));
    producer.close();
    let consumed;
    for (let i = 0; i < 50; i++) {
      consumed = await native.consumed();
      if (consumed) break;
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    return Response.json({ ignored, cancelled: reason.name, result, consumed });
  `);
  assert.deepEqual(await (await runtime.dispatchFetch('https://main.example')).json(), {
    ignored: 'done', cancelled: 'AbortError', result: 'queued', consumed: [8, 9],
  });
});

test('development bindings expose streaming HTTP fetch and reject service WebSocket upgrades', { timeout: 10_000 }, async t => {
  const runtime = await service(t, `
    const response = await native.fetch('https://native.example/echo', { method: 'POST', body: new Uint8Array([3, 4]) });
    let upgrade;
    try { await native.fetch('https://native.example/_tokamak/rpc', { headers: { upgrade: 'websocket' } }); }
    catch (error) { upgrade = error.name; }
    return Response.json({ bytes: [...new Uint8Array(await response.arrayBuffer())], upgrade });
  `);
  assert.deepEqual(await (await runtime.dispatchFetch('https://main.example')).json(), {
    bytes: [3, 4], upgrade: 'TypeError',
  });
});

test('ending the caller invocation cancels its remaining RPC streams', { timeout: 10_000 }, async t => {
  const runtime = await service(t, `
    if (new URL(request.url).pathname === '/status') return Response.json({ cancelled: await native.producerCancelled() });
    await native.produce();
    return new Response('finished');
  `);
  assert.equal(await (await runtime.dispatchFetch('https://main.example')).text(), 'finished');
  let state;
  for (const deadline = Date.now() + 1000; Date.now() < deadline;) {
    state = await (await runtime.dispatchFetch('https://main.example/status')).json();
    if (state.cancelled) break;
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  assert.deepEqual(state, { cancelled: true });
});
