function headers(value) {
  return [...value].filter(([name]) => name !== 'set-cookie')
    .concat(value.getSetCookie().map(cookie => ['set-cookie', cookie]));
}

export function encodeRequest(request) {
  return {
    url: request.url, method: request.method, headers: headers(request.headers),
    body: request.body, redirect: request.redirect, cf: request.cf,
  };
}

export function decodeRequest(value) {
  return new Request(value.url, { ...value, duplex: 'half' });
}

export function encodeResponse(response) {
  if (!(response instanceof Response)) throw new TypeError('Service fetch must return a Response');
  if (response.status === 101) throw new TypeError('Service binding WebSocket upgrades are not supported');
  return {
    status: response.status, statusText: response.statusText,
    headers: headers(response.headers), body: response.body,
  };
}

export function decodeResponse(value) {
  return new Response(value.body, value);
}
