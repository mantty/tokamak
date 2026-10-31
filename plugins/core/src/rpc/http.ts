import { dataCloneError, field } from './values.ts';

/** A request's parts, as RPC values carry them. */
export interface RequestParts {
  url: string;
  method: string;
  headers: [string, string][];
  body: ReadableStream<Uint8Array> | null;
  redirect: string;
  cf: RequestInit['cf'];
}

/** A response's parts, as RPC values carry them. */
export interface ResponseParts {
  status: number;
  statusText: string;
  headers: [string, string][];
  body: ReadableStream<Uint8Array> | null;
}

function headers(value: Headers): [string, string][] {
  return [...value].filter(([name]) => name !== 'set-cookie')
    .concat(value.getSetCookie().map((cookie): [string, string] => ['set-cookie', cookie]));
}

/** Whether a peer's `value` has a body the transport carries; `Request` and `Response` check the other parts. */
function hasBody(value: unknown): boolean {
  const body = field(value, 'body');
  return body === null || body instanceof ReadableStream;
}

export function encodeRequest(request: Request): RequestParts {
  return {
    url: request.url, method: request.method, headers: headers(request.headers),
    body: request.body, redirect: request.redirect, cf: request.cf,
  };
}

function isRequestParts(value: unknown): value is RequestParts {
  return hasBody(value) && typeof field(value, 'url') === 'string';
}

export function decodeRequest(value: unknown): Request {
  if (!isRequestParts(value)) throw dataCloneError();
  return new Request(value.url, value);
}

export function encodeResponse(response: unknown): ResponseParts {
  if (!(response instanceof Response)) throw new TypeError('Service fetch must return a Response');
  if (response.status === 101) throw new TypeError('Service binding WebSocket upgrades are not supported');
  return {
    status: response.status, statusText: response.statusText,
    headers: headers(response.headers), body: response.body,
  };
}

function isResponseParts(value: unknown): value is ResponseParts {
  return hasBody(value) && typeof field(value, 'status') === 'number';
}

/** The response parts a peer's `value` holds. */
export function responseParts(value: unknown): ResponseParts {
  if (!isResponseParts(value)) throw dataCloneError();
  return value;
}

export function decodeResponse(value: unknown): Response {
  const parts = responseParts(value);
  return new Response(parts.body, parts);
}
