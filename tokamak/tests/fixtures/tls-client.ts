import fs from 'node:fs'
import net from 'node:net'
import tls from 'node:tls'

const [port, host, authority, certificate, key] = process.argv.slice(2)
const fail = (error) => {
  console.error(error instanceof Error ? error.message : error)
  process.exit(1)
}
const socket = net.connect({ host: '127.0.0.1', port: Number(port) })
let response = Buffer.alloc(0)

socket.once('connect', () => {
  socket.write(`CONNECT ${host}:443 HTTP/1.1\r\nHost: ${host}:443\r\n\r\n`)
})

socket.on('data', (chunk) => {
  response = Buffer.concat([response, chunk])
  const end = response.indexOf('\r\n\r\n')
  if (end === -1) return
  if (!response.subarray(0, end).toString().startsWith('HTTP/1.1 200')) {
    fail(response.subarray(0, end).toString())
  }
  socket.pause()
  socket.removeAllListeners('data')
  const remainder = response.subarray(end + 4)
  if (remainder.length > 0) socket.unshift(remainder)
  const options = { socket, servername: host, ca: fs.readFileSync(authority) }
  if (certificate) options.cert = fs.readFileSync(certificate)
  if (key) options.key = fs.readFileSync(key)
  const secure = tls.connect(options)
  secure.once('secureConnect', () => {
    secure.write(`GET / HTTP/1.1\r\nHost: ${host}\r\nConnection: close\r\n\r\n`)
  })
  secure.once('error', fail)
  secure.once('close', () => fail('TLS connection closed before the worker responded'))
  let body = Buffer.alloc(0)
  secure.on('data', (chunk) => {
    body = Buffer.concat([body, chunk])
    const end = body.indexOf('\r\n\r\n')
    if (end === -1) return
    const response = body.subarray(0, end).toString()
    if (!response.startsWith('HTTP/1.1 204')) fail(response)
    process.exit(0)
  })
})

socket.once('error', fail)
socket.once('close', () => fail('TLS connection closed before handshake'))
setTimeout(() => fail('TLS connection timed out'), 5000).unref()
