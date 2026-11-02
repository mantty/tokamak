import { encode, decode, dataCloneError, field } from './values.ts';
import { takeStream, receiveStream, type StreamSource } from './streams.ts';
import { packFrame, unpackFrame } from './frames.ts';

/** The parts of a WebSocket event that a channel reads. */
interface SocketEvent {
  readonly type: string;
  readonly data?: unknown;
  readonly error?: unknown;
}

type SocketEventType = 'open' | 'message' | 'close' | 'error';

/** A WebSocket, or an object with the same interface, that carries one invocation. */
export interface RpcSocket {
  binaryType?: string;
  readonly readyState: number;
  addEventListener(type: SocketEventType, listener: (event: SocketEvent) => void): void;
  removeEventListener(type: SocketEventType, listener: (event: SocketEvent) => void): void;
  send(frame: ArrayBuffer): unknown;
  close(code?: number): void;
}

/** A peer's reply to an invocation, a read or a cancel: its value, or the error that failed it. */
export interface Settlement {
  failed: boolean;
  value: unknown;
}

/** The settlement that a peer's `reply` describes. */
export function settlement(reply: unknown): Settlement {
  const failed = field(reply, 'failed');
  if (typeof failed !== 'boolean') throw dataCloneError();
  return { failed, value: field(reply, 'value') };
}

/** A promise and the functions that settle it; its rejection is never reported as unhandled. */
class Deferred<T> {
  declare readonly promise: Promise<T>;
  declare resolve: (value: T) => void;
  declare reject: (reason: unknown) => void;

  constructor() {
    this.promise = new Promise((resolve, reject) => { this.resolve = resolve; this.reject = reject; });
    this.promise.catch(() => {});
  }
}

/** A local stream that the peer reads. */
interface Source {
  port: StreamSource;
  reading: boolean;
  cancelled: boolean;
  done: boolean;
  cleanup?: Promise<void>;
}

/** A peer's stream that this side reads. */
interface Port {
  reading: Deferred<unknown> | null;
  cancelling: Deferred<unknown> | null;
  done: boolean;
  terminal?: Settlement;
}

function cancelSource(source: Source, reason: unknown): Promise<void> {
  source.cancelled = true;
  source.cleanup ??= Promise.resolve().then(() => source.port.cancel(reason));
  return source.cleanup;
}

/** The settlement of reading `port`, and whether the read ended its stream. */
async function readSource(port: StreamSource): Promise<[Settlement, boolean]> {
  try {
    const read = await port.read();
    return [{ failed: false, value: read }, read.done];
  } catch (error) {
    return [{ failed: true, value: error }, true];
  }
}

/** The settlement of a peer's read or cancel `message` for `port`; a terminal cancel repeats the read that ended the stream. */
function portSettlement(port: Port, type: 'read' | 'cancelled', message: unknown): Settlement {
  if (type !== 'cancelled' || field(message, 'terminal') !== true) return settlement(message);
  if (!port.terminal) throw dataCloneError();
  return port.terminal.failed ? port.terminal : { failed: false, value: undefined };
}

/** Whether a peer's read `result` ended its stream. */
function ends(result: Settlement): boolean {
  return result.failed || Boolean(field(result.value, 'done'));
}

/** One invocation's values and demand-driven byte streams over a WebSocket. */
export class Channel {
  readonly #socket: RpcSocket;
  readonly #ready = new Deferred<void>();
  readonly #received = new Deferred<unknown>();
  readonly #closed = new Deferred<unknown>();
  readonly #outgoing = new Map<number, Source>();
  readonly #incoming = new Map<number, Port>();
  readonly #pending = new Set<Promise<unknown>>();
  #nextStream = 0;
  #sent = false;
  #delivered = false;
  #completed = false;
  #ended = false;
  #failure: unknown;

  /** The peer's value: the invocation for a callee, or its result for a caller. */
  readonly receive = this.#received.promise;
  /** Resolves with the reason the channel closed once its streams' cleanup settles. */
  readonly closed = this.#closed.promise;

  constructor(socket: RpcSocket) {
    this.#socket = socket;
    socket.binaryType = 'arraybuffer';
    socket.addEventListener('open', this.#onOpen);
    socket.addEventListener('message', this.#onMessage);
    socket.addEventListener('close', this.#onClose);
    socket.addEventListener('error', this.#onClose);
    if (socket.readyState === 1) this.#onOpen();
    if (socket.readyState > 1) this.close();
  }

  readonly close = (reason: unknown = new DOMException('RPC connection closed.', 'AbortError')): void => {
    if (this.#ended) return;
    this.#ended = true;
    this.#failure = reason;
    this.#ready.reject(reason);
    this.#received.reject(reason);
    for (const port of this.#incoming.values()) {
      port.reading?.reject(reason);
      port.cancelling?.reject(reason);
    }
    const cleanup = [...this.#outgoing.values()].map(source => cancelSource(source, reason));
    this.#incoming.clear();
    this.#outgoing.clear();
    this.#socket.removeEventListener('open', this.#onOpen);
    this.#socket.removeEventListener('message', this.#onMessage);
    this.#socket.removeEventListener('close', this.#onClose);
    this.#socket.removeEventListener('error', this.#onClose);
    try { this.#socket.close(1000); } catch { /* The transport can already be closed. */ }
    void Promise.allSettled(cleanup).then(() => { this.#closed.resolve(reason); });
  };

  /** Keep the channel open until `promise` settles. */
  waitUntil(promise: Promise<unknown>): void {
    const task = Promise.resolve(promise);
    this.#pending.add(task);
    const settled = () => { this.#pending.delete(task); this.#finish(); };
    task.then(settled, settled);
  }

  // The callee ends unused input streams after its work and result streams settle.
  complete(): void {
    this.#completed = true;
    this.#finish();
  }

  async send(value: unknown): Promise<void> {
    if (this.#sent || this.#completed) throw new TypeError('This RPC invocation has already sent its value.');
    const frame = this.#prepare('value', undefined, value);
    this.#sent = true;
    await this.#transmit(frame);
  }

  #finish(): void {
    if (this.#completed && this.#outgoing.size === 0 && this.#pending.size === 0) this.close();
  }

  #prepare(type: string, id: number | undefined, value: unknown): ArrayBuffer {
    if (this.#ended) throw this.#failure;
    const encoded = encode(value);
    const ids = encoded.streams.map(() => this.#nextStream++);
    const frame = packFrame({ type, id, data: encoded.data, streams: ids }, encoded.buffers);
    const acquired: Source[] = [];
    try {
      for (const stream of encoded.streams) acquired.push({ port: takeStream(stream), reading: false, cancelled: false, done: false });
    } catch (error) {
      for (const source of acquired) cancelSource(source, error).catch(() => {});
      throw error;
    }
    ids.forEach((stream, index) => this.#outgoing.set(stream, acquired[index]));
    return frame;
  }

  async #transmit(frame: ArrayBuffer): Promise<void> {
    await this.#ready.promise;
    if (this.#ended) throw this.#failure;
    try { await this.#socket.send(frame); }
    catch (error) { this.close(error); throw error; }
  }

  #write(type: string, id: number, value?: unknown): Promise<void> {
    return this.#transmit(this.#prepare(type, id, value));
  }

  #reply(type: string, id: number, result: Settlement): Promise<void> {
    try { return this.#write(type, id, result); }
    catch { return this.#write(type, id, { failed: true, value: dataCloneError() }); }
  }

  #remoteStream(id: number): ReadableStream<Uint8Array> {
    if (this.#incoming.has(id)) throw dataCloneError();
    const port: Port = { reading: null, cancelling: null, done: false };
    this.#incoming.set(id, port);
    return receiveStream({
      read: () => this.#readPort(port, id),
      cancel: reason => this.#cancelPort(port, id, reason),
    });
  }

  async #readPort(port: Port, id: number): Promise<unknown> {
    if (this.#ended) throw this.#failure;
    if (port.done || port.cancelling) return { done: true };
    if (port.reading) throw dataCloneError();
    port.reading = new Deferred();
    this.#write('pull', id).catch(this.close);
    return port.reading.promise;
  }

  async #cancelPort(port: Port, id: number, reason: unknown): Promise<unknown> {
    if (this.#ended) throw this.#failure;
    if (port.done) return;
    if (port.cancelling) return port.cancelling.promise;
    port.cancelling = new Deferred();
    port.reading?.resolve({ done: true });
    port.reading = null;
    try { this.#write('cancel', id, reason).catch(this.close); }
    catch (error) { this.close(error); }
    return port.cancelling.promise;
  }

  /** Whether reading or cancelling `source` no longer matters: the peer cancelled it, or the channel closed. */
  #abandoned(source: Source): boolean {
    return source.cancelled || this.#ended;
  }

  async #pull(id: number): Promise<void> {
    const source = this.#outgoing.get(id);
    if (!source || source.reading || source.cancelled || source.done) throw dataCloneError();
    source.reading = true;
    const [result, done] = await readSource(source.port);
    if (this.#abandoned(source)) return;
    source.reading = false;
    source.done = done;
    await this.#reply('read', id, result);
  }

  async #cancel(id: number, reason: unknown): Promise<void> {
    const source = this.#outgoing.get(id);
    if (source?.done) {
      source.cancelled = true;
      await this.#write('cancelled', id, { terminal: true });
      this.#outgoing.delete(id);
      this.#finish();
      return;
    }
    let result: Settlement = { failed: false, value: undefined };
    try { if (source) await cancelSource(source, reason); }
    catch (error) { result = { failed: true, value: error }; }
    if (this.#ended) return;
    await this.#reply('cancelled', id, result);
    this.#outgoing.delete(id);
    this.#finish();
  }

  #release(id: number): void {
    const source = this.#outgoing.get(id);
    if (!source?.done || source.cancelled) throw dataCloneError();
    this.#outgoing.delete(id);
    this.#finish();
  }

  #settle(type: 'read' | 'cancelled', id: number, message: unknown): void {
    const port = this.#incoming.get(id);
    if (!port) throw dataCloneError();
    const result = portSettlement(port, type, message);
    if (type === 'read' && port.cancelling) {
      if (ends(result)) port.terminal = result;
      return;
    }
    const waiting = type === 'read' ? port.reading : port.cancelling;
    if (!waiting) throw dataCloneError();
    if (result.failed) waiting.reject(result.value);
    else waiting.resolve(result.value);
    port.reading = null;
    if (type === 'cancelled' || ends(result)) this.#endPort(port, type, id);
  }

  /** Mark `port` done, and forget it once a read's end is acknowledged. */
  #endPort(port: Port, type: 'read' | 'cancelled', id: number): void {
    port.done = true;
    const acknowledgement = type === 'read' ? this.#write('release', id) : Promise.resolve();
    acknowledgement.then(() => { this.#incoming.delete(id); this.#finish(); }).catch(this.close);
  }

  #receiveValue(value: unknown): void {
    if (this.#delivered) throw dataCloneError();
    this.#delivered = true;
    this.#received.resolve(value);
  }

  readonly #onMessage = (event: SocketEvent): void => {
    try {
      const frame = unpackFrame(event.data);
      const value = decode({ ...frame, streams: frame.streams.map(id => this.#remoteStream(id)) });
      if (frame.type === 'value') {
        this.#receiveValue(value);
        return;
      }
      if (frame.id === undefined) throw dataCloneError();
      switch (frame.type) {
        case 'pull': this.#pull(frame.id).catch(this.close); break;
        case 'cancel': this.#cancel(frame.id, value).catch(this.close); break;
        case 'release': this.#release(frame.id); break;
        case 'read': case 'cancelled': this.#settle(frame.type, frame.id, value); break;
        default: throw dataCloneError();
      }
    } catch (error) { this.close(error); }
  };

  readonly #onOpen = (): void => { this.#ready.resolve(); };
  readonly #onClose = (event: SocketEvent): void => { this.close(event.error); };
}
