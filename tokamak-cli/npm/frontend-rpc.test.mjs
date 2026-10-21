import assert from 'node:assert/strict';
import { once } from 'node:events';
import fs from 'node:fs';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import test from 'node:test';
import { WebSocketServer } from 'ws';
import { createClient } from '../../plugins/core/src/rpc/client.mjs';
import { serveRpc } from '../../plugins/core/src/rpc/endpoint.mjs';

async function service(t, worker, methods) {
  const server = new WebSocketServer({ port: 0, host: '127.0.0.1' });
  const sessions = new Set();
  server.on('connection', socket => {
    const session = serveRpc(socket, ctx => ({ ...worker, ctx }), methods, { waitUntil() {} });
    sessions.add(session);
    session.finally(() => sessions.delete(session));
  });
  t.after(async () => {
    for (const socket of server.clients) socket.terminate();
    await new Promise(resolve => server.close(resolve));
    await Promise.all(sessions);
  });
  await once(server, 'listening');
  return createClient(methods, `ws://127.0.0.1:${server.address().port}/_tokamak/app`);
}

test('frontend RPC returns copied values and errors through real WebSockets', async t => {
  const client = await service(t, {
    echo(value) { return value; },
    fail() { throw Object.assign(new Error('Permission denied'), { code: 'DENIED' }); },
    unsupported() { return { callback() {} }; },
    badError() { throw Object.assign(new Error('bad'), { callback() {} }); },
  }, ['echo', 'fail', 'unsupported', 'badError']);
  const value = { map: new Map(), bytes: new Uint8Array([8, 9]), integer: 12n };
  value.map.set(value, value);
  assert.deepEqual(await client.echo(value), value);
  await assert.rejects(client.fail(), { message: 'Permission denied', code: 'DENIED' });
  await assert.rejects(client.unsupported(), { name: 'DataCloneError' });
  await assert.rejects(client.badError(), { name: 'DataCloneError' });
});

test('frontend RPC forwards byte streams without collecting them and propagates cancellation', async t => {
  const cancelled = Promise.withResolvers();
  const client = await service(t, {
    echo(stream) { return stream; },
  }, ['echo']);
  let pulls = 0;
  const source = new ReadableStream({
    type: 'bytes',
    pull(controller) {
      pulls++;
      controller.enqueue(new Uint8Array(150_000).fill(4));
    },
    cancel(reason) { cancelled.resolve(reason); },
  });
  const result = await client.echo(source);
  assert.equal(pulls, 0);
  const reader = result.getReader();
  const chunk = await reader.read();
  assert.equal(chunk.value.byteLength, 64 * 1024);
  assert.ok(chunk.value.every(value => value === 4));
  assert.equal(pulls, 1);
  await reader.cancel('page no longer needs data');
  assert.equal(await cancelled.promise, 'page no longer needs data');
});

test('frontend RPC dispatches only declared operations', async t => {
  const server = new WebSocketServer({ port: 0, host: '127.0.0.1' });
  let invoked = false;
  server.on('connection', socket => void serveRpc(socket, () => ({
    secret() { invoked = true; },
  }), [], { waitUntil() {} }));
  t.after(async () => {
    for (const socket of server.clients) socket.terminate();
    await new Promise(resolve => server.close(resolve));
  });
  await once(server, 'listening');
  const client = createClient(['secret'], `ws://127.0.0.1:${server.address().port}`);
  await assert.rejects(client.secret(), { name: 'TypeError', message: 'Unknown RPC method: secret' });
  assert.equal(invoked, false);
});

test('a rejected RPC result retains ownership of streams in its error data', async t => {
  const client = await service(t, {
    fail() {
      throw Object.assign(new Error('Details attached'), {
        details: new ReadableStream({
          type: 'bytes',
          start(controller) { controller.enqueue(new Uint8Array([7])); controller.close(); },
        }),
      });
    },
  }, ['fail']);
  let error;
  try { await client.fail(); } catch (failure) { error = failure; }
  assert.equal(error.message, 'Details attached');
  assert.deepEqual(new Uint8Array(await new Response(error.details).arrayBuffer()), new Uint8Array([7]));
});

test('a worker HTTP endpoint accepts same-origin RPC and rejects unrelated origins', { timeout: 15_000 }, async t => {
  const require = createRequire(import.meta.resolve('@cloudflare/vite-plugin'));
  const { Miniflare, convertV4MiniflareOptions } = await import(pathToFileURL(require.resolve('miniflare')).href);
  const files = ['endpoint', 'channel', 'frames', 'values', 'streams'];
  const modules = files.map(name => ({
    type: 'ESModule', path: `/rpc/${name}.mjs`,
    contents: fs.readFileSync(new URL(`../../plugins/core/src/rpc/${name}.mjs`, import.meta.url), 'utf8'),
  }));
  const miniflare = new Miniflare(convertV4MiniflareOptions({
    modulesRoot: '/',
    modules: [{
      type: 'ESModule', path: '/worker.mjs',
      contents: `import { rpcResponse } from './rpc/endpoint.mjs';
        export default { fetch(request, env, ctx) {
          return rpcResponse(request, () => ({
            echo(value) { return value; },
            stream() { return new ReadableStream({
              type: 'bytes',
              start(controller) { controller.enqueue(new Uint8Array([3, 4])); controller.close(); }
            }); }
          }), ['echo', 'stream'], ctx);
        } };`,
    }, ...modules],
    compatibilityDate: '2026-09-01',
  }));
  t.after(() => miniflare.dispose());
  const url = await miniflare.ready;
  const client = createClient(['echo', 'stream'], url.href);
  assert.deepEqual(await client.echo({ value: 3n }), { value: 3n });
  assert.deepEqual(new Uint8Array(await new Response(await client.stream()).arrayBuffer()), new Uint8Array([3, 4]));
  assert.equal((await miniflare.dispatchFetch(url)).status, 426);
  assert.equal((await miniflare.dispatchFetch(url, { method: 'POST' })).status, 405);
  assert.equal((await miniflare.dispatchFetch(url, { headers: { origin: 'https://unrelated.example' } })).status, 403);
});

test('settled RPC calls cancel input streams the handler does not retain', { timeout: 5000 }, async t => {
  const client = await service(t, {
    ignore() { return 'finished'; },
    fail() { throw new Error('finished'); },
  }, ['ignore', 'fail']);
  for (const method of ['ignore', 'fail']) {
    const cancelled = Promise.withResolvers();
    const source = new ReadableStream({ type: 'bytes', cancel: cancelled.resolve });
    if (method === 'ignore') assert.equal(await client.ignore(source), 'finished');
    else await assert.rejects(client.fail(source), { message: 'finished' });
    assert.equal((await cancelled.promise).name, 'AbortError');
  }
});

test('registered background work retains an argument stream after the RPC result', { timeout: 5000 }, async t => {
  const consumed = Promise.withResolvers();
  const client = await service(t, {
    consume(stream) {
      this.ctx.waitUntil(new Response(stream).arrayBuffer().then(consumed.resolve, consumed.reject));
      return 'queued';
    },
  }, ['consume']);
  let producer, cancelled = false;
  const source = new ReadableStream({
    type: 'bytes',
    start(controller) { producer = controller; },
    cancel() { cancelled = true; },
  });
  assert.equal(await client.consume(source), 'queued');
  assert.equal(cancelled, false);
  producer.enqueue(new Uint8Array([5, 6]));
  producer.close();
  assert.deepEqual(new Uint8Array(await consumed.promise), new Uint8Array([5, 6]));
});

test('work registered by a returned stream retains arguments after that stream settles', { timeout: 5000 }, async t => {
  for (const action of ['pull', 'cancel']) {
    const consumed = Promise.withResolvers();
    consumed.promise.catch(() => {});
    const client = await service(t, {
      consume(input) {
        const start = () => this.ctx.waitUntil(new Response(input).arrayBuffer().then(consumed.resolve, consumed.reject));
        return new ReadableStream({
          type: 'bytes',
          pull(controller) {
            if (action !== 'pull') return;
            start();
            controller.close();
            controller.byobRequest?.respond(0);
          },
          cancel() { if (action === 'cancel') start(); },
        });
      },
    }, ['consume']);
    let producer;
    const source = new ReadableStream({ type: 'bytes', start(controller) { producer = controller; } });
    const output = await client.consume(source);
    if (action === 'pull') assert.equal((await output.getReader().read()).done, true);
    else await output.cancel();
    producer.enqueue(new Uint8Array([6, 7]));
    producer.close();
    assert.deepEqual(new Uint8Array(await consumed.promise), new Uint8Array([6, 7]));
  }
});
