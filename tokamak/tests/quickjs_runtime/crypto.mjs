import * as nodeCrypto from "node:crypto";

function outcome(callback) {
  try { return callback(); }
  catch (error) { return { error: error.name, code: error.code ?? null }; }
}
async function asyncOutcome(callback) {
  try { return await callback(); }
  catch (error) { return { error: error.name, code: error.code ?? null }; }
}

export async function cryptoContracts() {
  const result = {};
  result.hashes = Object.fromEntries(nodeCrypto.getHashes().map(name => [name,
    outcome(() => nodeCrypto.createHash(name).update("abc").digest("hex")),
  ]));
  result.hmacAlgorithms = Object.fromEntries(nodeCrypto.getHashes().map(name => [name,
    outcome(() => nodeCrypto.createHmac(name, "key").update("a").update(new Uint8Array([98])).update("c").digest("hex")),
  ]));
  result.emptyHmacKeys = Object.fromEntries(nodeCrypto.getHashes().map(name => [name,
    ["", new Uint8Array()].map(key => outcome(() => nodeCrypto.createHmac(name, key).update("a").update("").update("bc").digest("hex"))),
  ]));
  result.hashConstruction = ["missing", "SHA-256", "sha224", undefined, null, 123].map(name => outcome(() => {
    nodeCrypto.createHash(name);
    return true;
  }));
  result.hashCopy = outcome(() => {
    const source = nodeCrypto.createHash("sha256").update("a");
    const fork = source.copy();
    return [source.update("bc").digest("hex"), fork.update("d").digest("hex")];
  });
  result.hashStream = await asyncOutcome(async () => {
    const hash = nodeCrypto.createHash("sha256");
    const output = [];
    const complete = new Promise((resolve, reject) => {
      hash.on("data", chunk => output.push(chunk.toString("hex")));
      hash.on("end", resolve);
      hash.on("error", reject);
    });
    hash.end("abc");
    await complete;
    return output;
  });
  result.hmacStream = await asyncOutcome(async () => {
    const hash = nodeCrypto.createHmac("sha256", "secret");
    const output = [];
    const complete = new Promise((resolve, reject) => {
      hash.on("data", chunk => output.push(chunk.toString("hex")));
      hash.on("end", resolve);
      hash.on("error", reject);
    });
    hash.end("abc");
    await complete;
    return output;
  });
  result.hmacRepeatedDigest = outcome(() => {
    const hash = nodeCrypto.createHmac("sha256", "secret").update("abc");
    const first = hash.digest("hex");
    return { first, second: hash.digest("hex"), update: outcome(() => hash.update("x")) };
  });
  result.digestMutation = await asyncOutcome(async () => {
    const stream = new crypto.DigestStream("SHA-256");
    const input = new TextEncoder().encode("ab");
    const writer = stream.getWriter();
    await writer.write(input);
    input.fill(0);
    await writer.write(new Uint8Array([99]));
    await writer.write(new Uint8Array());
    await writer.close();
    return { digest: [...new Uint8Array(await stream.digest)], count: String(stream.bytesWritten) };
  });
  return result;
}
