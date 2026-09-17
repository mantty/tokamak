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
  result.webCrypto = await webCryptoContracts();
  result.node = nodeCryptoContracts();
  return result;
}

const fixtures = {"rsaPrivateJwk":{"kty":"RSA","n":"jv04g0luIKANQemjF6MQqKsE4_ILCXgqsB0dXJEy82xmopockdyGvsvmWZVBO7uA1N2VEqBxjBrvaHU61WQKbJNIromdrdnwT6a0kSOrW1xrStndwFSL7oJDB13HKdolu7Q8a5eKYQarU-XcUJCBXs4yMArxa7tM9zzSNu-_EkqIL6C9IVvxcFqBj8xyNvESpaRIoduZcstZiFQJdxjoOwAuW7SRrFiK_O8ZuusBmAePnMylwLYWss5bHlVxSIOVEsIW9L8iotFHOTKiwF4a7S2xpQ8zpOCKfDPzLbdvfD4h-2PvKX_uWcGqZdlXKBEyeesFM7ubqURbvKMkT6zMnw","e":"AQAB","d":"Huknx93eZEglRYvz2V7Dcary1jITZ7smA0dv-vxaltvmvhzxsyiIqoNaqyAEZ5zLp3i1Sr8LfN2vxpWdH9dOF5WpXy3Zu-UCub1QiJW86_WpLhe2A-djDq7zPYrszKPfh1nZu-qZHAt8ixkETRhIF04c9FzRPthRNZtc2EpwtEu8tF8cljFC3sKYLOa-pEB_uR7P0ktKAqfFXZ0mA7fcxPBQJSZlXEKiqPy8njtaMYhORaLGOFhODROayKkQ7fsRIEGHmhJSZNpRAUIbTlhCMzFsU0ZKNQCjW6H_uX5tZNxxTc4F83pPaR5ShFqYZGhGDR5pxHFzclEPOHhJy8B1ZQ","p":"wqZj5l5ZI8srT85O9wuj-YDr1K-mWjPMcapY1Pb3ZD-lnHSQxpIdoqwPNFE3Ji0bPw_2XNKoNCTzFX1ZiWAD3uR1cHaxmAiOoZp2KZ0FzWLPDNCEgVgHXcdXXsb-lGTmOc-rpoIoz_i-0cXvQsDN5aSipeBUedXNcgt9bHFXlU0","q":"vA6A8-cEaQ1D9Ri0RGQ1ZoYmmMUYbgCBl9t6d2fEMhp30F6yMvjo9GAHGuiGKR5jv6t0fHZHqgaYIx7LuMzTUUG4qdxLvxLFb87DVFTRcK_Feiod8gRf0C01D0h_gVNa8IqiFv1UFcLV13JsHNLfec0UbK6IVj2rBAJ8tYu7g5s","dp":"kHewU62Y2VEUn1HPF9qC5E7EOgH4JKCnT4GQFtgJu1Tl1N5LCaYu6qprSngwx1vZChAN2Mzc3H7EECINz0D8_nRvmX3ux5kqS1T5-F67jLmWVLt6bQlpxjeKaCSnlHniyeuRSa73HYxQDB-tOc0hxBxSP2zlJdwCdG-EsnTY_U0","dq":"UajSohaMublC6ykBDjmdXpmeJPRg-VNK8tAhS7xJW6BWqqqUIsInFgakzzBtIWnK0q329Ry_XbtjUMzMlcCLeltZfpjkY2IZTcWw1-vEznPlAnlLa44utM0Mn0hR5ax2bsEkRWtXmeNyzA0pmRKQa-l7lv3qwdghbKpP0N2OXUU","qi":"p6aEAkLzd6zrfr9pXyac8uZy0wMrRjesWSGtA98yfLDUe3Pgq4DshAVmSHAxf_d3coC1IfV00cyx56RA2oSavA7gfjRZ8fD5gepwclI41EHAIxcdS4aiAcivlgxbLiXmFo6JwhyNzntx6fheibilNG7qLYHOzFpMm3VY_MAFohE"},"rsaPublicJwk":{"kty":"RSA","n":"jv04g0luIKANQemjF6MQqKsE4_ILCXgqsB0dXJEy82xmopockdyGvsvmWZVBO7uA1N2VEqBxjBrvaHU61WQKbJNIromdrdnwT6a0kSOrW1xrStndwFSL7oJDB13HKdolu7Q8a5eKYQarU-XcUJCBXs4yMArxa7tM9zzSNu-_EkqIL6C9IVvxcFqBj8xyNvESpaRIoduZcstZiFQJdxjoOwAuW7SRrFiK_O8ZuusBmAePnMylwLYWss5bHlVxSIOVEsIW9L8iotFHOTKiwF4a7S2xpQ8zpOCKfDPzLbdvfD4h-2PvKX_uWcGqZdlXKBEyeesFM7ubqURbvKMkT6zMnw","e":"AQAB"},"rsaPrivatePem":"-----BEGIN PRIVATE KEY-----\nMIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCO/TiDSW4goA1B\n6aMXoxCoqwTj8gsJeCqwHR1ckTLzbGaimhyR3Ia+y+ZZlUE7u4DU3ZUSoHGMGu9o\ndTrVZApsk0iuiZ2t2fBPprSRI6tbXGtK2d3AVIvugkMHXccp2iW7tDxrl4phBqtT\n5dxQkIFezjIwCvFru0z3PNI2778SSogvoL0hW/FwWoGPzHI28RKlpEih25lyy1mI\nVAl3GOg7AC5btJGsWIr87xm66wGYB4+czKXAthayzlseVXFIg5USwhb0vyKi0Uc5\nMqLAXhrtLbGlDzOk4Ip8M/Mtt298PiH7Y+8pf+5Zwapl2VcoETJ56wUzu5upRFu8\noyRPrMyfAgMBAAECggEAHuknx93eZEglRYvz2V7Dcary1jITZ7smA0dv+vxaltvm\nvhzxsyiIqoNaqyAEZ5zLp3i1Sr8LfN2vxpWdH9dOF5WpXy3Zu+UCub1QiJW86/Wp\nLhe2A+djDq7zPYrszKPfh1nZu+qZHAt8ixkETRhIF04c9FzRPthRNZtc2EpwtEu8\ntF8cljFC3sKYLOa+pEB/uR7P0ktKAqfFXZ0mA7fcxPBQJSZlXEKiqPy8njtaMYhO\nRaLGOFhODROayKkQ7fsRIEGHmhJSZNpRAUIbTlhCMzFsU0ZKNQCjW6H/uX5tZNxx\nTc4F83pPaR5ShFqYZGhGDR5pxHFzclEPOHhJy8B1ZQKBgQDCpmPmXlkjyytPzk73\nC6P5gOvUr6ZaM8xxqljU9vdkP6WcdJDGkh2irA80UTcmLRs/D/Zc0qg0JPMVfVmJ\nYAPe5HVwdrGYCI6hmnYpnQXNYs8M0ISBWAddx1dexv6UZOY5z6umgijP+L7Rxe9C\nwM3lpKKl4FR51c1yC31scVeVTQKBgQC8DoDz5wRpDUP1GLREZDVmhiaYxRhuAIGX\n23p3Z8QyGnfQXrIy+Oj0YAca6IYpHmO/q3R8dkeqBpgjHsu4zNNRQbip3Eu/EsVv\nzsNUVNFwr8V6Kh3yBF/QLTUPSH+BU1rwiqIW/VQVwtXXcmwc0t95zRRsrohWPasE\nAny1i7uDmwKBgQCQd7BTrZjZURSfUc8X2oLkTsQ6AfgkoKdPgZAW2Am7VOXU3ksJ\npi7qqmtKeDDHW9kKEA3YzNzcfsQQIg3PQPz+dG+Zfe7HmSpLVPn4XruMuZZUu3pt\nCWnGN4poJKeUeeLJ65FJrvcdjFAMH605zSHEHFI/bOUl3AJ0b4SydNj9TQKBgFGo\n0qIWjLm5QuspAQ45nV6ZniT0YPlTSvLQIUu8SVugVqqqlCLCJxYGpM8wbSFpytKt\n9vUcv127Y1DMzJXAi3pbWX6Y5GNiGU3FsNfrxM5z5QJ5S2uOLrTNDJ9IUeWsdm7B\nJEVrV5njcswNKZkSkGvpe5b96sHYIWyqT9Ddjl1FAoGBAKemhAJC83es636/aV8m\nnPLmctMDK0Y3rFkhrQPfMnyw1Htz4KuA7IQFZkhwMX/3d3KAtSH1dNHMseekQNqE\nmrwO4H40WfHw+YHqcHJSONRBwCMXHUuGogHIr5YMWy4l5haOicIcjc57cen4Xom4\npTRu6i2BzsxaTJt1WPzABaIR\n-----END PRIVATE KEY-----\n","rsaPublicPem":"-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAjv04g0luIKANQemjF6MQ\nqKsE4/ILCXgqsB0dXJEy82xmopockdyGvsvmWZVBO7uA1N2VEqBxjBrvaHU61WQK\nbJNIromdrdnwT6a0kSOrW1xrStndwFSL7oJDB13HKdolu7Q8a5eKYQarU+XcUJCB\nXs4yMArxa7tM9zzSNu+/EkqIL6C9IVvxcFqBj8xyNvESpaRIoduZcstZiFQJdxjo\nOwAuW7SRrFiK/O8ZuusBmAePnMylwLYWss5bHlVxSIOVEsIW9L8iotFHOTKiwF4a\n7S2xpQ8zpOCKfDPzLbdvfD4h+2PvKX/uWcGqZdlXKBEyeesFM7ubqURbvKMkT6zM\nnwIDAQAB\n-----END PUBLIC KEY-----\n","rsaPssSignature":"5c61178defc8e4a37fc1f99d3632c555d20a36b882acb2f10acb086adc13187c05f3dd529c3487d71dfd2d116002a655f632bb4c903b1c7cc24629afda2ae5bad816d169349cc3ab7b180ffa74a0ab43786822aa003d31afd71274bb6e13a712cc763edfa844897402dc85950ded1d7d5e0ebcb691914058068c5f526e36576485388ae85cd56952946c836b6d3d6ac73f89b23e6aebcb13c0ac301e7ee988e4545533624f8d66b98305607d685057be6b7ec484ac9d27cbbacf899e7dd3f80c38cef182209b7edbc7f11f627b4c6f501db2d540a825a0021c55aca7c925b75b486810fd0540571c389a56d5e95bc285049e59cbb952415c7f845429d5ee0439","rsaOaepCiphertext":"26c555ab6b8cdc0ae1461aa63ac6bd1288be1763f1a67737a5bf2b3da30ceea6a673c20616980aba92b8919554c9b4a3d4650f94da78334d5e828187c5ab5f7524e9c61717b307cacf1d0fbf08438e4d0e029cbf58cde616215d10851babd2edb6a68bc16f8c7ca290b9927bdbbe2c6650cdccf0629ca862b4f050a2d006a92b74900567b61324dad182c61dafac6e306c7aafaa41d69eb241831ae5a5302e8c03ad655572428858086906e5d5198cc8367e29f4f4cd9abac11e8356cab7017cdf7b3dab984017a353b79d821af93ec850e6901c0bc95c111cc4c132534aa722e6d902f5fb5505ef0d1e6e8411b80cab74fed2b681783ae3c0d7c64bb55b39db","rsaPublicEncrypt":"8bacc10ccc925b08ba580bb820c862c34e3b64a503400438219fde3d68401cb796b79845079a42c1e7fcbf752d249946db1e7dd7ec69e4058247bd869c55b16364b84790a7c411ad1076bcb1ba1b52c13a65bb7a396b0bc2006dea7079befbef88bdb4fb91db3fabebb0acd686d03acda689724ec6438558d4b97f732b1ea60cd6fae0dc78f3ca8e558bc86d9a367729b01ad9d98f4637611b315d76dac5904057609f8cfd57d2c577bce3ebf0f97f207878920daa3e80482a692fe0f98dd78bad56efaa72293ce11e3f1bfd36e486bf69dc79627fe44813784eb1c74199629d4c1d7ec63d29239889e5addecd21f50a19e207d6f1f8768e6fd39c2abe54ae68","ecP-256":{"a":{"kty":"EC","x":"_MZcRBQBf3P3UXxV9TkEJ37qpikycCDKsBsS2Cx_Ag8","y":"2UYLvDCQEwTOCKf3cccJQg0ytP8pblYd3jsk9qfeG84","crv":"P-256","d":"nMn6mC714AM544pSqLig9MV5wnGy8UV7JCudwQEnbg0"},"b":{"kty":"EC","x":"6mOJtfbSMuJPUPR3kvC5LBM3fjMd4jEGiCZ5RWN4pUI","y":"FxlvFOp19fhITHktidbAKfWbjvtP4lCgqt-z0w_kcxc","crv":"P-256","d":"bmbqliynYmPJKyBRXfjti9VClv2yudYLQEEXOMbyZ8Y"},"signature":"aa64f0b773ca7b5d43c8eb35c8195ab4f98b52a6a6346bc7048c3497cc1dd12c2fec7f34217b987d15e654531f843748d90ec39ebed8eca476855c65f4e8eb10"},"ecP-384":{"a":{"kty":"EC","x":"d0ee2cL9IhsRRvvR3RdkfoeQ8eBBPjDUmY6VO4FRFXB1VptuG0IfEaWwDumXHboq","y":"i04hwWhWT7kG3V3DWt3Le9DpJRD9tkkBoSAqOeaGkNj6IZV8XLAnYc2C_PA2RzF6","crv":"P-384","d":"0bDWVTpdvRcPL7gdW2JWjhcSJSkPUevXhwvNrvLVZygOEAQgVHsQQL-GZxU9vdzO"},"b":{"kty":"EC","x":"NkKqP10EKxt6PJXw3crXG7mV5Z-I5nGlP8z3hGxqemBTqy8FR0B2K5b7Y-aG5_H3","y":"tZp0N4bJElN4Bh5Tjk3hTlziy4WbORCsAFb7zqBicYyAC7UzLsRVGQLg3hoKqwY1","crv":"P-384","d":"Nm0NnqZtPQihBkft038BLZ96BUPuurZyxrRvXmYxocCPvpUB56JXSO601B1rujJL"},"signature":"5b88ff9fcb7ebe4958cbdabc70bc245056bc207c3c19a8119e159f9cad6129e7f9f7634d11a4ecce89d1c4c63f2013ff4eb9228bafc079c3050e99a7f07810d334a31bdf9d9c25a9061c527dc62a7d5c622c631a4344a7c7c270621cf1897c61"},"ecP-521":{"a":{"kty":"EC","x":"ABvWSnVYMxFWJiG077IJ7lnrAcBBlvk7rJkyLue34fCg_U69qeUqjHvv1hCiztXrWGy7FmnDBen4FcAgQNQ9oroZ","y":"Aetc7IFv6PYLURthb3RvKCHgmv5E3EGDW_YdFf3s6SqHPws22UGF6J_6yRwj9YvQPyrQKPg9NtWjhxc_6pZ8zsgM","crv":"P-521","d":"ACrkADQioZ6bcgi9DwZ6zWtIxn0_wM7Ms84iMzByh3MAWmI4Ff7bG9lOQp66hKtYaVyE49zM1VLFHnUJ2w67YPXj"},"b":{"kty":"EC","x":"AV9F18kQMJbS1RCmPS378KambvyFKJNkFf8dTcrWB-lQeEjKdCn28nrgGG3eBjxAmCfEd1a_VC03MjrxQFSA9lYQ","y":"Aek8uRYV5ebORsbtv1mRnNxZ5YLZzG5IXlH0o7X1aWWC0lAZrwkVbY9fr1D2bpPwVtpQY1eszb7pZbM07cU68BpU","crv":"P-521","d":"AQg1sQpT3gxh9ZcHRROXcpBckdnTi-AiIHgbpkCR8ZVdWTWVpmAdKr4Et6xkF-v4_633mwlR3BwPFbn8xZr8osHg"},"signature":"01a086c74476a32e57aa2210518fb98501d151dc6ee87ad2265a3b7b20e314a3bb3c757efa8b5e06ca0c679c53cc9014b0b9bcfbe78a7de4cad1ef62a00f3abff43800f044cba0f47d20e21f9343957fb50fa4e4ab508d6d8afe95a4c7878b70c192b188e02c778e9affa5fdd854d03dd4296bcc7a8d06f1f880b058f76622716aa7e68a"},"ed25519Jwk":{"crv":"Ed25519","d":"ahV441A6ToRRIVmAvepDAwsdorKHTK80dDltyqN3jOo","x":"Km00Uwd1zuZ1L_DWqR4ZrfBFiuozQan7pWVnaCILvZc","kty":"OKP"},"x25519":{"a":{"crv":"X25519","d":"6D0lLbl2Rhx52FM9fXFXYjuT6iU1HIIKGPb_MNY2_l8","x":"p4EgXxTojlPidoXCpvNNsUUoo6UV86bTjQH3Ozrhnk0","kty":"OKP"},"b":{"crv":"X25519","d":"-JZw_34ZDQRid0DuN7AW4P9x5WHdUZjW0r8C1uIDYlU","x":"LLIg1LyW3AtSu463h4WnVAMmQpddhTDSAtCiHvDj5C4","kty":"OKP"}}};
const encoder = new TextEncoder();
const text = value => encoder.encode(value);
const hex = buffer => [...new Uint8Array(buffer)].map(byte => byte.toString(16).padStart(2, "0")).join("");
const fromHex = value => new Uint8Array(value.match(/../g).map(byte => parseInt(byte, 16)));
const definedKeys = jwk => Object.keys(jwk).filter(key => jwk[key] !== undefined).sort();
const publicJwk = jwk => Object.fromEntries(Object.entries(jwk).filter(([key]) => !["d", "p", "q", "dp", "dq", "qi"].includes(key)));

async function webCryptoContracts() {
  const result = {};
  const subtle = crypto.subtle;
  result.digests = {};
  for (const name of ["SHA-1", "SHA-256", "SHA-384", "SHA-512"]) {
    result.digests[name] = hex(await subtle.digest(name, text("abc")));
  }
  const hmacKey = await subtle.importKey("raw", text("key"), { name: "HMAC", hash: "SHA-256" }, true, ["sign", "verify"]);
  const hmacSignature = await subtle.sign("HMAC", hmacKey, text("abc"));
  result.hmac = {
    signature: hex(hmacSignature),
    valid: await subtle.verify("HMAC", hmacKey, hmacSignature, text("abc")),
    invalid: await subtle.verify("HMAC", hmacKey, hmacSignature, text("abd")),
    jwk: await subtle.exportKey("jwk", hmacKey),
  };
  const pbkdfKey = await subtle.importKey("raw", text("password"), "PBKDF2", false, ["deriveBits"]);
  result.pbkdf2 = {};
  for (const hash of ["SHA-1", "SHA-256", "SHA-512"]) {
    result.pbkdf2[hash] = hex(await subtle.deriveBits({ name: "PBKDF2", salt: text("salt"), iterations: 1000, hash }, pbkdfKey, 256));
  }
  const hkdfKey = await subtle.importKey("raw", fromHex("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b"), "HKDF", false, ["deriveBits"]);
  result.hkdf = {
    rfc5869: hex(await subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt: fromHex("000102030405060708090a0b0c"), info: fromHex("f0f1f2f3f4f5f6f7f8f9") }, hkdfKey, 336)),
    emptySalt: hex(await subtle.deriveBits({ name: "HKDF", hash: "SHA-384", salt: new Uint8Array(), info: text("info") }, hkdfKey, 128)),
    tooLong: await asyncOutcome(() => subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt: new Uint8Array(), info: new Uint8Array() }, hkdfKey, 255 * 32 * 8 + 8)),
  };
  const aesRaw = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
  const iv = fromHex("cafebabefacedbaddecaf888");
  const gcmKey = await subtle.importKey("raw", aesRaw, "AES-GCM", true, ["encrypt", "decrypt"]);
  const gcmCiphertext = await subtle.encrypt({ name: "AES-GCM", iv, additionalData: text("aad") }, gcmKey, text("hello, world"));
  const gcmShort = await subtle.encrypt({ name: "AES-GCM", iv, tagLength: 96 }, gcmKey, text("hello"));
  const tampered = new Uint8Array(gcmCiphertext.slice(0)); tampered[0] ^= 1;
  result.aesGcm = {
    ciphertext: hex(gcmCiphertext),
    shortTag: hex(gcmShort),
    longIv: hex(await subtle.encrypt({ name: "AES-GCM", iv: fromHex("9313225df88406e555909c5aff5269aa6a7a9538534f7da1e4c303d2a318a728c3c0c95156809539fcf0e2429a6b525416aedbf5a0de6a57a637b39b") }, gcmKey, text("long iv"))),
    plaintext: hex(await subtle.decrypt({ name: "AES-GCM", iv, additionalData: text("aad") }, gcmKey, gcmCiphertext)),
    tampered: await asyncOutcome(() => subtle.decrypt({ name: "AES-GCM", iv, additionalData: text("aad") }, gcmKey, tampered)),
    wrongAad: await asyncOutcome(() => subtle.decrypt({ name: "AES-GCM", iv }, gcmKey, gcmCiphertext)),
    badTagLength: await asyncOutcome(() => subtle.encrypt({ name: "AES-GCM", iv, tagLength: 100 }, gcmKey, text("x"))),
    emptyIv: await asyncOutcome(() => subtle.encrypt({ name: "AES-GCM", iv: new Uint8Array() }, gcmKey, text("x"))),
  };
  const cbcKey = await subtle.importKey("raw", aesRaw.slice(0, 16), "AES-CBC", true, ["encrypt", "decrypt"]);
  const cbcIv = fromHex("000102030405060708090a0b0c0d0e0f");
  const cbcCiphertext = await subtle.encrypt({ name: "AES-CBC", iv: cbcIv }, cbcKey, text("The quick brown fox jumps over the lazy dog"));
  const cbcCorrupt = new Uint8Array(cbcCiphertext.slice(0)); cbcCorrupt[cbcCorrupt.length - 1] ^= 1;
  result.aesCbc = {
    ciphertext: hex(cbcCiphertext),
    empty: hex(await subtle.encrypt({ name: "AES-CBC", iv: cbcIv }, cbcKey, new Uint8Array())),
    plaintext: hex(await subtle.decrypt({ name: "AES-CBC", iv: cbcIv }, cbcKey, cbcCiphertext)),
    corrupt: await asyncOutcome(() => subtle.decrypt({ name: "AES-CBC", iv: cbcIv }, cbcKey, cbcCorrupt)),
    badIv: await asyncOutcome(() => subtle.encrypt({ name: "AES-CBC", iv: cbcIv.slice(0, 8) }, cbcKey, text("x"))),
  };
  const ctrKey = await subtle.importKey("raw", aesRaw.slice(0, 16), "AES-CTR", true, ["encrypt", "decrypt"]);
  const counter = fromHex("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff");
  result.aesCtr = {
    full: hex(await subtle.encrypt({ name: "AES-CTR", counter, length: 128 }, ctrKey, fromHex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51"))),
    wrap: hex(await subtle.encrypt({ name: "AES-CTR", counter: fromHex("000000000000000000000000fffffffe"), length: 8 }, ctrKey, new Uint8Array(64))),
    overflow: await asyncOutcome(() => subtle.encrypt({ name: "AES-CTR", counter, length: 1 }, ctrKey, new Uint8Array(48))),
  };
  const kek = await subtle.importKey("raw", fromHex("000102030405060708090a0b0c0d0e0f"), "AES-KW", true, ["wrapKey", "unwrapKey"]);
  const wrapped = await subtle.wrapKey("raw", cbcKey, kek, "AES-KW");
  const unwrapped = await subtle.unwrapKey("raw", wrapped, kek, "AES-KW", "AES-CBC", true, ["encrypt"]);
  result.aesKw = { wrapped: hex(wrapped), unwrapped: hex(await subtle.exportKey("raw", unwrapped)) };

  const rsaPrivate = await subtle.importKey("jwk", { ...fixtures.rsaPrivateJwk, alg: "RS256" }, { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" }, true, ["sign"]);
  const rsaPublic = await subtle.importKey("jwk", { ...fixtures.rsaPublicJwk, alg: "RS256" }, { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" }, true, ["verify"]);
  const rsaSignature = await subtle.sign("RSASSA-PKCS1-v1_5", rsaPrivate, text("abc"));
  const pkcs8 = await subtle.exportKey("pkcs8", rsaPrivate);
  const spki = await subtle.exportKey("spki", rsaPublic);
  const rsaReimport = await subtle.importKey("pkcs8", pkcs8, { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" }, true, ["sign"]);
  result.rsaPkcs1 = {
    signature: hex(rsaSignature),
    valid: await subtle.verify("RSASSA-PKCS1-v1_5", rsaPublic, rsaSignature, text("abc")),
    invalid: await subtle.verify("RSASSA-PKCS1-v1_5", rsaPublic, rsaSignature, text("abd")),
    pkcs8: hex(pkcs8),
    spki: hex(spki),
    reimported: await subtle.exportKey("jwk", rsaReimport),
    spkiJwk: await subtle.exportKey("jwk", await subtle.importKey("spki", spki, { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" }, true, ["verify"])),
  };
  const pssPrivate = await subtle.importKey("jwk", { ...fixtures.rsaPrivateJwk, alg: "PS256" }, { name: "RSA-PSS", hash: "SHA-256" }, true, ["sign"]);
  const pssPublic = await subtle.importKey("jwk", { ...fixtures.rsaPublicJwk, alg: "PS256" }, { name: "RSA-PSS", hash: "SHA-256" }, true, ["verify"]);
  const pssSignature = await subtle.sign({ name: "RSA-PSS", saltLength: 32 }, pssPrivate, text("abc"));
  result.rsaPss = {
    length: pssSignature.byteLength,
    roundTrip: await subtle.verify({ name: "RSA-PSS", saltLength: 32 }, pssPublic, pssSignature, text("abc")),
    reference: await subtle.verify({ name: "RSA-PSS", saltLength: 32 }, pssPublic, fromHex(fixtures.rsaPssSignature), text("abc")),
    wrongSalt: await subtle.verify({ name: "RSA-PSS", saltLength: 20 }, pssPublic, fromHex(fixtures.rsaPssSignature), text("abc")),
  };
  const oaepPrivate = await subtle.importKey("jwk", { ...fixtures.rsaPrivateJwk, alg: "RSA-OAEP-256" }, { name: "RSA-OAEP", hash: "SHA-256" }, true, ["decrypt"]);
  const oaepPublic = await subtle.importKey("jwk", { ...fixtures.rsaPublicJwk, alg: "RSA-OAEP-256" }, { name: "RSA-OAEP", hash: "SHA-256" }, true, ["encrypt"]);
  const oaepCiphertext = await subtle.encrypt({ name: "RSA-OAEP", label: text("label") }, oaepPublic, text("secret message"));
  result.rsaOaep = {
    length: oaepCiphertext.byteLength,
    roundTrip: hex(await subtle.decrypt({ name: "RSA-OAEP", label: text("label") }, oaepPrivate, oaepCiphertext)),
    reference: hex(await subtle.decrypt({ name: "RSA-OAEP", label: text("label") }, oaepPrivate, fromHex(fixtures.rsaOaepCiphertext))),
    wrongLabel: await asyncOutcome(() => subtle.decrypt({ name: "RSA-OAEP" }, oaepPrivate, fromHex(fixtures.rsaOaepCiphertext))),
  };

  result.ecdsa = {};
  result.ecdh = {};
  for (const curve of ["P-256", "P-384", "P-521"]) {
    const pair = fixtures[`ec${curve}`];
    const signKey = await subtle.importKey("jwk", pair.a, { name: "ECDSA", namedCurve: curve }, true, ["sign"]);
    const verifyKey = await subtle.importKey("jwk", publicJwk(pair.a), { name: "ECDSA", namedCurve: curve }, true, ["verify"]);
    const signature = await subtle.sign({ name: "ECDSA", hash: "SHA-256" }, signKey, text("abc"));
    const raw = await subtle.exportKey("raw", verifyKey);
    result.ecdsa[curve] = {
      length: signature.byteLength,
      roundTrip: await subtle.verify({ name: "ECDSA", hash: "SHA-256" }, verifyKey, signature, text("abc")),
      reference: await subtle.verify({ name: "ECDSA", hash: "SHA-256" }, verifyKey, fromHex(pair.signature), text("abc")),
      wrongHash: await subtle.verify({ name: "ECDSA", hash: "SHA-384" }, verifyKey, fromHex(pair.signature), text("abc")),
      wrongMessage: await subtle.verify({ name: "ECDSA", hash: "SHA-256" }, verifyKey, fromHex(pair.signature), text("abd")),
      raw: hex(raw),
      spki: hex(await subtle.exportKey("spki", verifyKey)),
      pkcs8: hex(await subtle.exportKey("pkcs8", signKey)),
      jwk: await subtle.exportKey("jwk", signKey),
      rawJwk: await subtle.exportKey("jwk", await subtle.importKey("raw", raw, { name: "ECDSA", namedCurve: curve }, true, ["verify"])),
      mismatchedJwk: await asyncOutcome(() => subtle.importKey("jwk", { ...pair.a, x: pair.b.x }, { name: "ECDSA", namedCurve: curve }, true, ["sign"])),
    };
    const dhPrivate = await subtle.importKey("jwk", pair.a, { name: "ECDH", namedCurve: curve }, true, ["deriveBits"]);
    const dhPublic = await subtle.importKey("jwk", publicJwk(pair.b), { name: "ECDH", namedCurve: curve }, true, []);
    result.ecdh[curve] = hex(await subtle.deriveBits({ name: "ECDH", public: dhPublic }, dhPrivate, { "P-256": 256, "P-384": 384, "P-521": 528 }[curve]));
  }
  const edPrivate = await subtle.importKey("jwk", fixtures.ed25519Jwk, "Ed25519", true, ["sign"]);
  const edPublic = await subtle.importKey("jwk", publicJwk(fixtures.ed25519Jwk), "Ed25519", true, ["verify"]);
  const edSignature = await subtle.sign("Ed25519", edPrivate, text("abc"));
  result.ed25519 = {
    signature: hex(edSignature),
    valid: await subtle.verify("Ed25519", edPublic, edSignature, text("abc")),
    invalid: await subtle.verify("Ed25519", edPublic, edSignature, text("abd")),
    raw: hex(await subtle.exportKey("raw", edPublic)),
    spki: hex(await subtle.exportKey("spki", edPublic)),
    pkcs8: hex(await subtle.exportKey("pkcs8", edPrivate)),
    jwk: { ...await subtle.exportKey("jwk", edPrivate), alg: undefined },
  };
  const xPrivate = await subtle.importKey("jwk", fixtures.x25519.a, "X25519", true, ["deriveBits"]);
  const xPublic = await subtle.importKey("jwk", publicJwk(fixtures.x25519.b), "X25519", true, []);
  result.x25519 = {
    shared: hex(await subtle.deriveBits({ name: "X25519", public: xPublic }, xPrivate, 256)),
    raw: hex(await subtle.exportKey("raw", xPublic)),
    spki: hex(await subtle.exportKey("spki", xPublic)),
    pkcs8: hex(await subtle.exportKey("pkcs8", xPrivate)),
    jwk: await subtle.exportKey("jwk", xPrivate),
  };

  const generated = await subtle.generateKey({ name: "RSA-PSS", modulusLength: 1024, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" }, true, ["sign", "verify"]);
  const generatedJwk = await subtle.exportKey("jwk", generated.privateKey);
  const generatedSignature = await subtle.sign({ name: "RSA-PSS", saltLength: 16 }, generated.privateKey, text("abc"));
  result.generated = {
    rsa: { n: generatedJwk.n.length, e: generatedJwk.e, fields: definedKeys(generatedJwk), verifies: await subtle.verify({ name: "RSA-PSS", saltLength: 16 }, generated.publicKey, generatedSignature, text("abc")) },
  };
  const ecPair = await subtle.generateKey({ name: "ECDSA", namedCurve: "P-384" }, true, ["sign", "verify"]);
  const ecSignature = await subtle.sign({ name: "ECDSA", hash: "SHA-512" }, ecPair.privateKey, text("abc"));
  result.generated.ec = { length: ecSignature.byteLength, verifies: await subtle.verify({ name: "ECDSA", hash: "SHA-512" }, ecPair.publicKey, ecSignature, text("abc")), fields: definedKeys(await subtle.exportKey("jwk", ecPair.privateKey)) };
  const edPair = await subtle.generateKey("Ed25519", true, ["sign", "verify"]);
  result.generated.ed25519 = await subtle.verify("Ed25519", edPair.publicKey, await subtle.sign("Ed25519", edPair.privateKey, text("abc")), text("abc"));
  const xPairA = await subtle.generateKey("X25519", true, ["deriveBits"]);
  const xPairB = await subtle.generateKey("X25519", true, ["deriveBits"]);
  const sharedA = hex(await subtle.deriveBits({ name: "X25519", public: xPairB.publicKey }, xPairA.privateKey, 256));
  const sharedB = hex(await subtle.deriveBits({ name: "X25519", public: xPairA.publicKey }, xPairB.privateKey, 256));
  result.generated.x25519 = { agree: sharedA === sharedB, length: sharedA.length };
  return result;
}

function nodeCryptoContracts() {
  const result = {};
  const key = Buffer.from("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f", "hex");
  const iv = Buffer.from("cafebabefacedbaddecaf888", "hex");
  const cipher = nodeCrypto.createCipheriv("aes-256-gcm", key, iv);
  cipher.setAAD(Buffer.from("aad"));
  const chunks = [cipher.update(Buffer.from("hello, ")), cipher.update(Buffer.from("world")), cipher.final()];
  const tag = cipher.getAuthTag();
  const decipher = nodeCrypto.createDecipheriv("aes-256-gcm", key, iv);
  decipher.setAAD(Buffer.from("aad"));
  decipher.setAuthTag(tag);
  result.gcm = {
    chunks: chunks.map(chunk => chunk.toString("hex")),
    tag: tag.toString("hex"),
    plaintext: Buffer.concat([decipher.update(Buffer.concat(chunks)), decipher.final()]).toString("utf8"),
    badTag: outcome(() => { const d = nodeCrypto.createDecipheriv("aes-256-gcm", key, iv); d.setAAD(Buffer.from("aad")); d.setAuthTag(Buffer.alloc(16)); d.update(Buffer.concat(chunks)); return d.final(); }),
    shortTag: outcome(() => { const c = nodeCrypto.createCipheriv("aes-256-gcm", key, iv, { authTagLength: 12 }); c.update(Buffer.from("x")); c.final(); return c.getAuthTag().toString("hex"); }),
  };
  const cbcIv = Buffer.from("000102030405060708090a0b0c0d0e0f", "hex");
  const cbc = nodeCrypto.createCipheriv("aes-128-cbc", key.subarray(0, 16), cbcIv);
  const cbcChunks = [cbc.update(Buffer.from("The quick brown fox jumps")), cbc.update(Buffer.from(" over the lazy dog")), cbc.final()];
  const unpadded = nodeCrypto.createCipheriv("aes-128-cbc", key.subarray(0, 16), cbcIv);
  unpadded.setAutoPadding(false);
  result.cbc = {
    chunks: cbcChunks.map(chunk => chunk.toString("hex")),
    plaintext: Buffer.concat([nodeCrypto.createDecipheriv("aes-128-cbc", key.subarray(0, 16), cbcIv).update(Buffer.concat(cbcChunks)), nodeCrypto.createDecipheriv("aes-128-cbc", key.subarray(0, 16), cbcIv).update(Buffer.concat(cbcChunks)).length ? Buffer.alloc(0) : Buffer.alloc(0)]).length,
    roundTrip: (() => { const d = nodeCrypto.createDecipheriv("aes-128-cbc", key.subarray(0, 16), cbcIv); return Buffer.concat([d.update(Buffer.concat(cbcChunks)), d.final()]).toString("utf8"); })(),
    unpadded: [unpadded.update(Buffer.alloc(32, 7)).toString("hex"), unpadded.final().toString("hex")],
    partialUnpadded: outcome(() => { const c = nodeCrypto.createCipheriv("aes-128-cbc", key.subarray(0, 16), cbcIv); c.setAutoPadding(false); c.update(Buffer.from("abc")); return c.final(); }),
    badPadding: outcome(() => { const d = nodeCrypto.createDecipheriv("aes-128-cbc", key.subarray(0, 16), cbcIv); d.update(Buffer.alloc(16, 1)); return d.final(); }),
  };
  const ctr = nodeCrypto.createCipheriv("aes-128-ctr", key.subarray(0, 16), Buffer.from("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff", "hex"));
  result.ctr = [ctr.update(Buffer.from("6bc1bee22e409f96e93d7e11", "hex")).toString("hex"), ctr.update(Buffer.from("7393172aae2d8a571e03ac9c9eb76fac45af8e51", "hex")).toString("hex"), ctr.final().toString("hex")];

  const privateKey = nodeCrypto.createPrivateKey(fixtures.rsaPrivatePem);
  const publicKey = nodeCrypto.createPublicKey(fixtures.rsaPublicPem);
  result.rsa = {
    privateJwk: privateKey.export({ format: "jwk" }),
    publicJwk: publicKey.export({ format: "jwk" }),
    sign: nodeCrypto.createSign("sha256").update(Buffer.from("abc")).sign(privateKey).toString("hex"),
    verify: nodeCrypto.createVerify("sha256").update(Buffer.from("abc")).verify(publicKey, nodeCrypto.sign("sha256", Buffer.from("abc"), privateKey)),
    privateEncrypt: outcome(() => nodeCrypto.privateEncrypt(fixtures.rsaPrivatePem, Buffer.from("legacy")).toString("hex")),
    publicDecrypt: outcome(() => nodeCrypto.publicDecrypt(fixtures.rsaPublicPem, nodeCrypto.privateEncrypt(fixtures.rsaPrivatePem, Buffer.from("legacy"))).toString("utf8")),
    oaepRoundTrip: outcome(() => nodeCrypto.privateDecrypt({ key: fixtures.rsaPrivatePem, oaepHash: "sha1" }, nodeCrypto.publicEncrypt({ key: fixtures.rsaPublicPem, oaepHash: "sha1" }, Buffer.from("legacy"))).toString("utf8")),
  };
  const ecdh = nodeCrypto.createECDH("prime256v1");
  ecdh.setPrivateKey(Buffer.from(fixtures["ecP-256"].a.d, "base64url"));
  const peer = nodeCrypto.createECDH("prime256v1");
  peer.setPrivateKey(Buffer.from(fixtures["ecP-256"].b.d, "base64url"));
  result.ecdh = {
    uncompressed: ecdh.getPublicKey("hex"),
    compressed: ecdh.getPublicKey("hex", "compressed"),
    secret: ecdh.computeSecret(peer.getPublicKey()).toString("hex"),
    converted: nodeCrypto.ECDH.convertKey(peer.getPublicKey(), "prime256v1", "buffer", "hex", "compressed"),
  };
  // RFC 2409 group 2 (1024-bit MODP).
  const modp2 = Buffer.from("FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE65381FFFFFFFFFFFFFFFF", "hex");
  const alice = nodeCrypto.createDiffieHellman(modp2, 2);
  alice.setPrivateKey(Buffer.from("0123456789abcdef0123456789abcdef", "hex"));
  const bob = nodeCrypto.createDiffieHellman(modp2, 2);
  bob.setPrivateKey(Buffer.from("fedcba9876543210fedcba9876543210", "hex"));
  result.dh = {
    alicePublic: alice.generateKeys("hex"),
    secret: alice.computeSecret(bob.generateKeys()).toString("hex"),
    symmetric: alice.computeSecret(bob.getPublicKey()).equals(bob.computeSecret(alice.getPublicKey())),
    generated: (() => { const p = nodeCrypto.createDiffieHellman(256); return { primeBits: p.getPrime().length * 8, generator: p.getGenerator("hex"), safe: nodeCrypto.checkPrimeSync(p.getPrime()) }; })(),
  };
  const toBuffer = value => { let digits = value.toString(16); if (digits.length % 2) digits = "0" + digits; return Buffer.from(digits, "hex"); };
  const toBigInt = buffer => BigInt("0x" + Buffer.from(buffer).toString("hex"));
  result.primes = {
    known: [2n, 3n, 4n, 97n, 561n, 7919n, 2n ** 61n - 1n, 2n ** 61n + 1n].map(value => nodeCrypto.checkPrimeSync(toBuffer(value))),
    generated: nodeCrypto.checkPrimeSync(nodeCrypto.generatePrimeSync(64)),
    generatedBits: toBigInt(nodeCrypto.generatePrimeSync(64)).toString(2).length,
    safe: (() => { const p = toBigInt(nodeCrypto.generatePrimeSync(64, { safe: true })); return nodeCrypto.checkPrimeSync(toBuffer(p)) && nodeCrypto.checkPrimeSync(toBuffer((p - 1n) / 2n)); })(),
    congruent: (() => { const p = toBigInt(nodeCrypto.generatePrimeSync(64, { add: toBuffer(12n), rem: toBuffer(11n) })); return [nodeCrypto.checkPrimeSync(toBuffer(p)), p % 12n]; })().map(String),
  };
  result.kdf = {
    scrypt: nodeCrypto.scryptSync("password", "NaCl", 64, { N: 1024, r: 8, p: 16 }).toString("hex"),
    scryptLimit: outcome(() => nodeCrypto.scryptSync("password", "salt", 16, { N: 2 ** 20, maxmem: 1024 })),
    pbkdf2: nodeCrypto.pbkdf2Sync("password", "salt", 2, 20, "sha1").toString("hex"),
    hkdf: Buffer.from(nodeCrypto.hkdfSync("sha256", "key", "salt", "info", 42)).toString("hex"),
  };
  const ecPair = nodeCrypto.generateKeyPairSync("ec", { namedCurve: "P-256" });
  result.generatedPairs = {
    ec: nodeCrypto.verify("sha256", Buffer.from("abc"), ecPair.publicKey, nodeCrypto.sign("sha256", Buffer.from("abc"), ecPair.privateKey)),
    ecJwkFields: definedKeys(ecPair.privateKey.export({ format: "jwk" })),
    rsa: (() => { const pair = nodeCrypto.generateKeyPairSync("rsa", { modulusLength: 1024 }); return nodeCrypto.verify("sha256", Buffer.from("abc"), pair.publicKey, nodeCrypto.sign("sha256", Buffer.from("abc"), pair.privateKey)); })(),
    ed25519: (() => { const pair = nodeCrypto.generateKeyPairSync("ed25519"); return nodeCrypto.verify(null, Buffer.from("abc"), pair.publicKey, nodeCrypto.sign(null, Buffer.from("abc"), pair.privateKey)); })(),
  };
  return result;
}
