import { encode, decode, dataCloneError } from './values.mjs';
import { takeStream, receiveStream } from './streams.mjs';
import { packFrame, unpackFrame } from './frames.mjs';

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  promise.catch(() => {});
  return { promise, resolve, reject };
}

function cancelSource(source, reason) {
  source.cancelled = true;
  source.cleanup ??= Promise.resolve().then(() => source.port.cancel(reason));
  return source.cleanup;
}

/** One invocation's values and demand-driven byte streams over a WebSocket. */
export function openChannel(socket) {
  const ready = deferred(), received = deferred(), closed = deferred();
  const outgoing = new Map(), incoming = new Map();
  const pending = new Set();
  let nextStream = 0, sent = false, delivered = false, completed = false, ended = false, failure;
  socket.binaryType = 'arraybuffer';

  function close(reason = new DOMException('RPC connection closed.', 'AbortError')) {
    if (ended) return;
    ended = true;
    failure = reason;
    ready.reject(reason);
    received.reject(reason);
    for (const port of incoming.values()) {
      port.reading?.reject(reason);
      port.cancelling?.reject(reason);
    }
    const cleanup = [...outgoing.values()].map(source => cancelSource(source, reason));
    incoming.clear();
    outgoing.clear();
    socket.removeEventListener('open', onOpen);
    socket.removeEventListener('message', onMessage);
    socket.removeEventListener('close', onClose);
    socket.removeEventListener('error', onClose);
    try { socket.close(1000); } catch { /* The transport can already be closed. */ }
    Promise.allSettled(cleanup).then(() => closed.resolve(reason));
  }

  function finish() {
    if (completed && outgoing.size === 0 && pending.size === 0) close();
  }

  function prepare(type, id, value) {
    if (ended) throw failure;
    const encoded = encode(value);
    const ids = encoded.streams.map(() => nextStream++);
    const frame = packFrame({ type, id, data: encoded.data, streams: ids }, encoded.buffers);
    const acquired = [];
    try {
      for (const stream of encoded.streams) acquired.push({ port: takeStream(stream), reading: false, cancelled: false });
    } catch (error) {
      for (const source of acquired) cancelSource(source, error).catch(() => {});
      throw error;
    }
    ids.forEach((stream, index) => outgoing.set(stream, acquired[index]));
    return frame;
  }

  async function transmit(frame) {
    await ready.promise;
    if (ended) throw failure;
    try { await socket.send(frame); }
    catch (error) { close(error); throw error; }
  }

  function write(type, id, value) {
    return transmit(prepare(type, id, value));
  }

  function reply(type, id, failed, value) {
    try { return write(type, id, { failed, value }); }
    catch { return write(type, id, { failed: true, value: dataCloneError() }); }
  }

  function remoteStream(id) {
    if (!Number.isSafeInteger(id) || id < 0 || incoming.has(id)) throw dataCloneError();
    const port = { reading: null, cancelling: null, done: false };
    incoming.set(id, port);
    return receiveStream({
      read() {
        if (ended) return Promise.reject(failure);
        if (port.done || port.cancelling) return Promise.resolve({ done: true });
        if (port.reading) throw dataCloneError();
        port.reading = deferred();
        write('pull', id).catch(close);
        return port.reading.promise;
      },
      cancel(reason) {
        if (ended) return Promise.reject(failure);
        if (port.done) return Promise.resolve();
        if (port.cancelling) return port.cancelling.promise;
        port.cancelling = deferred();
        port.reading?.resolve({ done: true });
        port.reading = null;
        try { write('cancel', id, reason).catch(close); }
        catch (error) { close(error); }
        return port.cancelling.promise;
      },
    });
  }

  async function pull(id) {
    const source = outgoing.get(id);
    if (!source || source.reading || source.cancelled || source.done) throw dataCloneError();
    source.reading = true;
    let value, failed = false;
    try { value = await source.port.read(); }
    catch (error) { value = error; failed = true; }
    if (source.cancelled || ended) return;
    source.reading = false;
    source.done = failed || value.done;
    await reply('read', id, failed, value);
  }

  async function cancel(id, reason) {
    const source = outgoing.get(id);
    if (source?.done) {
      source.cancelled = true;
      await write('cancelled', id, { terminal: true });
      outgoing.delete(id);
      finish();
      return;
    }
    let value, failed = false;
    try { if (source) await cancelSource(source, reason); }
    catch (error) { value = error; failed = true; }
    if (ended) return;
    await reply('cancelled', id, failed, value);
    outgoing.delete(id);
    finish();
  }

  function release(id) {
    const source = outgoing.get(id);
    if (!source?.done || source.cancelled) throw dataCloneError();
    outgoing.delete(id);
    finish();
  }

  function settle(type, id, result) {
    const port = incoming.get(id);
    if (!port) throw dataCloneError();
    if (type === 'cancelled' && result?.terminal === true) {
      if (!port.terminal) throw dataCloneError();
      result = port.terminal.failed ? port.terminal : { failed: false, value: undefined };
    }
    if (typeof result?.failed !== 'boolean') throw dataCloneError();
    if (type === 'read' && port.cancelling) {
      if (result.failed || result.value.done) port.terminal = result;
      return;
    }
    const waiting = type === 'read' ? port.reading : port.cancelling;
    if (!waiting) throw dataCloneError();
    if (result.failed) waiting.reject(result.value);
    else waiting.resolve(result.value);
    port.reading = null;
    if (type === 'cancelled' || result.failed || result.value.done) {
      port.done = true;
      const acknowledgement = type === 'read' ? write('release', id) : Promise.resolve();
      acknowledgement.then(() => { incoming.delete(id); finish(); }).catch(close);
    }
  }

  function onMessage(event) {
    try {
      const frame = unpackFrame(event.data);
      const streams = frame.streams.map(remoteStream);
      const value = decode({ ...frame, streams });
      switch (frame.type) {
        case 'value':
          if (delivered) throw dataCloneError();
          delivered = true;
          received.resolve(value);
          break;
        case 'pull': pull(frame.id).catch(close); break;
        case 'cancel': cancel(frame.id, value).catch(close); break;
        case 'release': release(frame.id); break;
        case 'read': case 'cancelled': settle(frame.type, frame.id, value); break;
        default: throw dataCloneError();
      }
    } catch (error) { close(error); }
  }

  function onOpen() { ready.resolve(); }
  function onClose(event) { close(event.error); }
  socket.addEventListener('open', onOpen);
  socket.addEventListener('message', onMessage);
  socket.addEventListener('close', onClose);
  socket.addEventListener('error', onClose);
  if (socket.readyState === 1) onOpen();
  if (socket.readyState > 1) close();
  return {
    receive: received.promise,
    closed: closed.promise,
    close,
    waitUntil(promise) {
      const task = Promise.resolve(promise);
      pending.add(task);
      const settled = () => { pending.delete(task); finish(); };
      task.then(settled, settled);
    },
    // The callee ends unused input streams after its work and result streams settle.
    complete() { completed = true; finish(); },
    async send(value) {
      if (sent || completed) throw new TypeError('This RPC invocation has already sent its value.');
      const frame = prepare('value', undefined, value);
      sent = true;
      await transmit(frame);
    },
  };
}
