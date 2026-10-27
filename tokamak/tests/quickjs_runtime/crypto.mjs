import * as nodeCrypto from "node:crypto";
import { asyncOutcome, outcome } from "./outcome.mjs";

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
  result.secretKeys = await secretKeyContracts();
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
  const exportedAlg = async (algorithm, usages, key) => {
    const generated = key ? await subtle.importKey("spki", key, algorithm, true, usages) : await subtle.generateKey(algorithm, true, usages);
    return (await subtle.exportKey("jwk", generated)).alg;
  };
  result.jwkAlgorithms = await Promise.all([
    [{ name: "HMAC", hash: "SHA-1" }, ["sign"]], [{ name: "HMAC", hash: "SHA-512" }, ["sign"]],
    [{ name: "AES-GCM", length: 128 }, ["encrypt"]], [{ name: "AES-CBC", length: 192 }, ["encrypt"]],
    [{ name: "AES-CTR", length: 256 }, ["encrypt"]], [{ name: "AES-KW", length: 128 }, ["wrapKey"]],
    [{ name: "RSASSA-PKCS1-v1_5", hash: "SHA-1" }, ["verify"], spki], [{ name: "RSA-PSS", hash: "SHA-384" }, ["verify"], spki],
    [{ name: "RSA-OAEP", hash: "SHA-512" }, ["encrypt"], spki],
  ].map(([algorithm, usages, key]) => exportedAlg(algorithm, usages, key)));
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

// HMAC, AES, HKDF and PBKDF2 keys through every operation, including the order of validation errors.
async function secretKeyContracts() {
  const subtle = crypto.subtle;
  const describe = value => {
    if (value instanceof CryptoKey) return { type: value.type, algorithm: value.algorithm, usages: value.usages, extractable: value.extractable };
    return value instanceof ArrayBuffer ? hex(value) : value;
  };
  const attempt = callback => asyncOutcome(async () => describe(await callback()));
  const all = async cases => Object.fromEntries(await Promise.all(Object.entries(cases).map(async ([name, callback]) => [name, await attempt(callback)])));
  const oct = (k, fields = {}) => ({ kty: "oct", k, ...fields });
  const k128 = "AAAAAAAAAAAAAAAAAAAAAA";
  const sha256 = { name: "HMAC", hash: "SHA-256" };
  const hmacRaw = (bytes, fields = {}, usages = ["sign"]) => () => subtle.importKey("raw", bytes, { ...sha256, ...fields }, true, usages);
  const hmacJwk = (jwk, fields = {}, usages = ["sign"], extractable = true) => () => subtle.importKey("jwk", jwk, { ...sha256, ...fields }, extractable, usages);
  const aesJwk = (jwk, usages = ["encrypt"], name = "AES-GCM") => () => subtle.importKey("jwk", jwk, name, true, usages);
  const exportRaw = make => async () => hex(await subtle.exportKey("raw", await make()));
  const ones = () => new Uint8Array([255, 255, 255]);
  const hkdf = (usages = ["deriveBits", "deriveKey"]) => subtle.importKey("raw", new Uint8Array(8), "HKDF", false, usages);
  const pbkdf2 = () => subtle.importKey("raw", new Uint8Array(8), "PBKDF2", false, ["deriveBits", "deriveKey"]);
  const hkdfParams = { name: "HKDF", hash: "SHA-256", salt: new Uint8Array(), info: new Uint8Array() };
  const pbkdf2Params = { name: "PBKDF2", hash: "SHA-256", salt: new Uint8Array(), iterations: 2 };
  const hkdfBits = (fields, length) => async () => subtle.deriveBits({ ...hkdfParams, ...fields }, await hkdf(), length);
  const pbkdf2Bits = (fields, length) => async () => subtle.deriveBits({ ...pbkdf2Params, ...fields }, await pbkdf2(), length);
  const hkdfKey = (derived, usages = ["sign"]) => async () => subtle.deriveKey(hkdfParams, await hkdf(), derived, true, usages);
  const hmacKey = (usages = ["sign", "verify"]) => subtle.importKey("raw", new Uint8Array([1, 2, 3]), sha256, true, usages);
  const aesKey = (name, usages = name === "AES-KW" ? ["wrapKey", "unwrapKey"] : ["encrypt", "decrypt", "wrapKey", "unwrapKey"], extractable = true) => subtle.importKey("raw", new Uint8Array(16), name, extractable, usages);
  const gcm = { name: "AES-GCM", iv: new Uint8Array(12) };
  const ctr = { name: "AES-CTR", counter: new Uint8Array(16), length: 64 };
  const gcmUnwrap = (text, algorithm, usages = ["sign"], format = "jwk") => async () => {
    const key = await aesKey("AES-GCM");
    return subtle.unwrapKey(format, await subtle.encrypt(gcm, key, text), key, gcm, algorithm, true, usages);
  };
  const result = {};
  result.usages = await all({
    order: hmacRaw(new Uint8Array(3), {}, ["verify", "sign", "verify"]),
    derivation: () => hkdf(["deriveBits", "deriveKey"]),
    generated: () => subtle.generateKey({ name: "AES-CBC", length: 192 }, false, ["wrapKey", "encrypt", "encrypt"]),
    stringified: hmacRaw(new Uint8Array(3), {}, [{ toString: () => "sign" }]),
    number: hmacRaw(new Uint8Array(3), {}, [1]),
    string: hmacRaw(new Uint8Array(3), {}, "sign"),
    set: hmacRaw(new Uint8Array(3), {}, new Set(["sign"])),
    missing: () => subtle.importKey("raw", new Uint8Array(3), sha256, true),
    aesWrap: () => Promise.all(["AES-CTR", "AES-CBC", "AES-GCM"].map(name => aesKey(name).then(describe))),
    kwEncrypt: () => aesKey("AES-KW", ["encrypt"]),
    beforeHash: () => subtle.importKey("raw", new Uint8Array(3), { name: "HMAC" }, false, ["encrypt"]),
    beforeKeyType: hmacJwk({ kty: "RSA" }, {}, ["encrypt"]),
    beforeKeyData: aesJwk(oct("@@"), ["sign"]),
    beforeFormat: () => subtle.importKey("spki", new Uint8Array(3), sha256, true, ["encrypt"]),
    generateBeforeLength: () => subtle.generateKey({ name: "AES-GCM", length: 100 }, false, ["sign"]),
    generateMissingLength: () => subtle.generateKey({ name: "AES-GCM" }, false, ["sign"]),
    generateHmacLength: () => subtle.generateKey({ ...sha256, length: 0 }, false, ["encrypt"]),
    generateAfterHash: () => subtle.generateKey({ name: "HMAC", hash: "SHA-999" }, false, ["encrypt"]),
  });
  result.emptyUsages = await all({
    hmac: hmacRaw(new Uint8Array(3), {}, []),
    hmacBadLength: hmacRaw(new Uint8Array(), {}, []),
    hmacBadHash: hmacRaw(new Uint8Array(3), { hash: "SHA-999" }, []),
    aes: () => aesKey("AES-GCM", []),
    aesKeyOps: aesJwk(oct(k128, { key_ops: [] }), []),
    hkdf: () => hkdf([]),
    pbkdf2: () => subtle.importKey("raw", new Uint8Array(3), "PBKDF2", false, []),
    generateHmac: () => subtle.generateKey(sha256, true, []),
    generateHmacLength: () => subtle.generateKey({ ...sha256, length: 0 }, true, []),
    generateAes: () => subtle.generateKey({ name: "AES-GCM", length: 128 }, true, []),
    generateAesLength: () => subtle.generateKey({ name: "AES-GCM", length: 100 }, true, []),
    derived: hkdfKey({ name: "AES-GCM", length: 128 }, []),
    unwrapped: gcmUnwrap(text('{"kty":"oct","k":"AAAA"}'), sha256, []),
    privateKey: async () => {
      const pair = await subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"]);
      return subtle.importKey("pkcs8", await subtle.exportKey("pkcs8", pair.privateKey), { name: "ECDSA", namedCurve: "P-256" }, true, []);
    },
  });
  result.formats = await all({
    upperRaw: () => subtle.importKey("RAW", new Uint8Array(3), sha256, true, ["sign"]),
    upperAes: () => subtle.importKey("RAW", new Uint8Array(16), "AES-GCM", true, ["encrypt"]),
    upperHkdf: () => subtle.importKey("RAW", new Uint8Array(3), "HKDF", false, ["sign"]),
    upperJwk: () => subtle.importKey("JWK", oct("AAAA"), sha256, true, ["sign"]),
    number: () => subtle.importKey(5, new Uint8Array(3), sha256, true, ["sign"]),
    spkiHmac: () => subtle.importKey("spki", new Uint8Array(3), sha256, true, ["sign"]),
    spkiHmacNoHash: () => subtle.importKey("spki", new Uint8Array(3), { name: "HMAC" }, true, ["sign"]),
    spkiAes: () => subtle.importKey("spki", new Uint8Array(16), "AES-GCM", true, ["encrypt"]),
    unknownWithObject: () => subtle.importKey("foo", oct("AAAA"), "FOO", true, ["sign"]),
    rawWithObject: () => subtle.importKey("raw", oct("AAAA"), "FOO", true, ["encrypt"]),
    jwkWithBytes: () => subtle.importKey("jwk", new Uint8Array(3), "HKDF", false, ["deriveBits"]),
    rawArrayBuffer: hmacRaw(new ArrayBuffer(3)),
    rawDataView: hmacRaw(new DataView(new ArrayBuffer(3))),
    upperSpki: () => subtle.importKey("SPKI", new Uint8Array(16), { name: "ECDSA", namedCurve: "P-256" }, true, ["verify"]),
  });
  result.algorithms = await all({
    lowerNames: () => subtle.importKey("raw", new Uint8Array(3), { name: "hmac", hash: "sha-256" }, true, ["sign"]),
    hashObject: hmacRaw(new Uint8Array(3), { hash: { name: "sha-384" } }),
    md5: hmacRaw(new Uint8Array(3), { hash: "MD5" }),
    md5Sign: async () => hex(await subtle.sign("HMAC", await hmacRaw(new Uint8Array(3), { hash: "md5" })(), new Uint8Array(1))),
    md5Jwk: async () => subtle.exportKey("jwk", await hmacRaw(new Uint8Array(3), { hash: "MD5" })()),
    sha224: hmacRaw(new Uint8Array(3), { hash: "SHA-224" }),
    hashNull: hmacRaw(new Uint8Array(3), { hash: null }),
    hashEmpty: hmacRaw(new Uint8Array(3), { hash: {} }),
    empty: () => subtle.importKey("raw", new Uint8Array(3), {}, true, ["sign"]),
    null: () => subtle.importKey("raw", new Uint8Array(3), null, true, ["sign"]),
    number: () => subtle.importKey("raw", new Uint8Array(3), 5, true, ["sign"]),
    symbolName: () => subtle.importKey("raw", new Uint8Array(3), { name: Symbol("HMAC") }, true, ["sign"]),
    unknown: () => subtle.importKey("jwk", oct("AAAA"), "FOO", false, ["sign"]),
    extFalseUnknown: () => subtle.importKey("jwk", { kty: "oct", ext: false }, "FOO", true, ["sign"]),
    aesIgnoresLength: () => subtle.importKey("raw", new Uint8Array(16), { name: "AES-GCM", length: 256, hash: "SHA-999" }, true, ["encrypt"]),
    aesHugeLength: () => subtle.importKey("raw", new Uint8Array(16), { name: "AES-GCM", length: 2 ** 40 }, true, ["encrypt"]),
    digestEmpty: () => subtle.digest({}, new Uint8Array()),
    digestNull: () => subtle.digest(null, new Uint8Array()),
    digestNumber: () => subtle.digest(5, new Uint8Array()),
    digestDataBeforeName: () => subtle.digest("SHA-999", "x"),
    digestStreamEmpty: () => new crypto.DigestStream({}),
    digestStreamNull: () => new crypto.DigestStream(null),
    digestStreamNumber: () => new crypto.DigestStream(5),
  });
  result.hmacLengths = await all(Object.fromEntries([24, 20, 17, 16, 25, 0, -1, 20.5, "20", true, NaN, null, Infinity, -Infinity, -0.5, 2 ** 31 - 1, 2 ** 31, -(2 ** 31) - 1, [20], { valueOf: () => 20 }]
    .map(length => [String(length), hmacRaw(new Uint8Array(3), { length })])));
  Object.assign(result.hmacLengths, await all({
    bigint: hmacRaw(new Uint8Array(3), { length: 20n }),
    symbol: hmacRaw(new Uint8Array(3), { length: Symbol("length") }),
    hugeBeforeUsages: hmacRaw(new Uint8Array(3), { length: 2 ** 40 }, ["encrypt"]),
    emptyBeforeHash: hmacRaw(new Uint8Array(), { hash: "SHA-999" }),
    mismatchBeforeHash: hmacRaw(new Uint8Array(3), { hash: "SHA-999", length: 8 }),
    jwk: hmacJwk(oct("AAAA"), { length: 20 }),
    zeroed20: exportRaw(hmacRaw(ones(), { length: 20 })),
    zeroed17: exportRaw(hmacRaw(ones(), { length: 17 })),
    zeroedJwk: async () => subtle.exportKey("jwk", await hmacRaw(ones(), { length: 20 })()),
    zeroedSign: async () => [
      hex(await subtle.sign("HMAC", await hmacRaw(ones(), { length: 20 })(), new Uint8Array(1))),
      hex(await subtle.sign("HMAC", await hmacRaw(new Uint8Array([255, 255, 240]))(), new Uint8Array(1))),
    ],
  }));
  result.hmacJwk = await all({
    ...Object.fromEntries([
      ["SHA-256", "HS256"], ["SHA-1", "HS1"], ["SHA-384", "HS384"], ["SHA-512", "HS512"], ["SHA-256", "HS512"], ["SHA-256", "hs256"],
      ["SHA-256", "XYZ"], ["SHA-512", ""], ["SHA-1", "HS160"], ["SHA-999", "HS256"], ["SHA-999", "HS999"], ["sha-256", "HS256"],
      ["sha-256", "XYZ"], ["MD5", "HS"], ["MD5", "HS5"], ["XXX-256", "HS256"], [{ name: "SHA-256" }, "HS256"], [{ name: "sha-256" }, "HS256"],
    ].map(([hash, alg]) => [`${JSON.stringify(hash)}:${alg}`, hmacJwk(oct("AAAA", { alg }), { hash })])),
    algBeforeLength: hmacJwk(oct("", { alg: "HS512" })),
    algAfterKeyType: hmacJwk({ kty: "RSA", alg: "HS512" }),
    algAfterKeyData: hmacJwk(oct("@@", { alg: "HS512" })),
    keyTypeBeforeHash: hmacJwk({ kty: "RSA" }, { hash: "SHA-999" }),
    keyDataBeforeHash: hmacJwk(oct("@@"), { hash: "SHA-999" }),
    noHash: () => subtle.importKey("jwk", { kty: "RSA" }, { name: "HMAC" }, false, ["sign"]),
    keyOpsIgnored: hmacJwk(oct("AAAA", { key_ops: ["verify", "verify"] })),
    useIgnored: hmacJwk(oct("AAAA", { use: "enc" })),
    noKeyType: hmacJwk({ k: "AAAA" }),
    keyTypeArray: hmacJwk({ kty: ["oct"], k: "AAAA" }),
    keyDataArray: hmacJwk({ kty: "oct", k: ["AAAA"] }),
    noKeyData: hmacJwk({ kty: "oct" }),
    numericKeyData: hmacJwk({ kty: "oct", k: 5 }),
    array: hmacJwk([]),
    null: hmacJwk(null),
    string: hmacJwk("oct"),
    inherited: aesJwk(Object.create(oct(k128))),
    keyOpsString: hmacJwk(oct("AAAA", { key_ops: "sign" })),
    keyOpsNull: hmacJwk(oct("AAAA", { key_ops: null })),
    othString: hmacJwk(oct("AAAA", { oth: "x" })),
    ...Object.fromEntries([false, "", null, 0, true, "no"].map(ext => [`ext:${JSON.stringify(ext)}`, hmacJwk(oct("AAAA", { ext }))])),
    extFalseNotExtractable: hmacJwk(oct("AAAA", { ext: false }), {}, ["sign"], false),
    extBeforeUsages: hmacJwk(oct("AAAA", { ext: false }), {}, ["encrypt"]),
    extBeforeKeyType: hmacJwk({ kty: "RSA", ext: false }),
  });
  result.base64url = await all(Object.fromEntries(["", "A", "AA", "AAA", "AAAA", "AA=", "AA==", "AAA=", "AAAA=", "AAAA==", "AB", "AB==", "A+A/", "-_-_", "A-A_", "/w", "_w", "__8",
    "A A A A", "AA\nAA", "AA\tAA", "AA\vAA", "AA\fAA", "AA\rAA", " AAAA ", "AA==AA", "AAA=A", "=AAA", "AAAA ", "AA AA=", "AA = =", "AA=\n=", "AAAAA", "AAAAAA", "AAAA.", "éAAA"]
    .map(k => [JSON.stringify(k), exportRaw(hmacJwk(oct(k)))])));
  result.aesJwk = await all({
    good: aesJwk(oct(k128)),
    alg: aesJwk(oct(k128, { alg: "A128GCM" })),
    algMismatch: aesJwk(oct(k128, { alg: "A256GCM" })),
    algLower: aesJwk(oct(k128, { alg: "a128gcm" })),
    algNumber: aesJwk(oct(k128, { alg: 5 })),
    algNull: aesJwk(oct(k128, { alg: null })),
    algCbc: aesJwk(oct(k128, { alg: "A128CBC" }), ["encrypt"], "AES-CBC"),
    algCbcMismatch: aesJwk(oct(k128, { alg: "A128GCM" }), ["encrypt"], "AES-CBC"),
    algCtr: aesJwk(oct(k128, { alg: "A128CTR" }), ["encrypt"], "AES-CTR"),
    algKw: aesJwk(oct("A".repeat(32), { alg: "A192KW" }), ["wrapKey"], "aes-kw"),
    useEnc: aesJwk(oct(k128, { use: "enc" })),
    useSig: aesJwk(oct(k128, { use: "sig" })),
    useEmpty: aesJwk(oct(k128, { use: "" })),
    useNull: aesJwk(oct(k128, { use: null })),
    useSigWithoutUsages: aesJwk(oct(k128, { use: "sig" }), []),
    keyOps: aesJwk(oct(k128, { key_ops: ["encrypt"] })),
    keyOpsExtra: aesJwk(oct(k128, { key_ops: ["encrypt", "decrypt", "sign"] })),
    keyOpsMissing: aesJwk(oct(k128, { key_ops: ["decrypt"] })),
    keyOpsPartial: aesJwk(oct(k128, { key_ops: ["encrypt"] }), ["encrypt", "decrypt"]),
    keyOpsEmpty: aesJwk(oct(k128, { key_ops: [] })),
    keyOpsDuplicate: aesJwk(oct(k128, { key_ops: ["encrypt", "encrypt"] })),
    keyOpsNumbers: aesJwk(oct(k128, { key_ops: [1] })),
    duplicateBeforeMissing: aesJwk(oct(k128, { key_ops: ["decrypt", "decrypt"] })),
    useBeforeKeyOps: aesJwk(oct(k128, { use: "sig", key_ops: ["decrypt"] })),
    algBeforeUse: aesJwk(oct(k128, { use: "sig", alg: "A256GCM" })),
    algBeforeKeyOps: aesJwk(oct(k128, { key_ops: ["decrypt"], alg: "A256GCM" })),
    lengthBeforeAlg: aesJwk(oct("AAAA", { alg: "A256GCM" })),
    lengthBeforeKeyOps: aesJwk(oct("AAAA", { key_ops: ["decrypt"] })),
    keyType: aesJwk({ kty: "RSA", use: "sig" }),
    noKeyData: aesJwk({ kty: "oct" }),
    badKeyData: aesJwk(oct("@@")),
    rawEmpty: () => subtle.importKey("raw", new Uint8Array(), "AES-GCM", true, ["encrypt"]),
    extFalse: aesJwk(oct(k128, { ext: false }), ["sign"]),
  });
  result.derivationImport = await all({
    hkdfExtractable: () => subtle.importKey("raw", new Uint8Array(3), "HKDF", true, ["deriveBits"]),
    pbkdf2Extractable: () => subtle.importKey("raw", new Uint8Array(3), "PBKDF2", true, ["deriveBits"]),
    extractableBeforeFormat: () => subtle.importKey("jwk", oct("AAAA"), "HKDF", true, ["deriveBits"]),
    usagesBeforeExtractable: () => subtle.importKey("raw", new Uint8Array(3), "PBKDF2", true, ["sign"]),
    usagesBeforeFormat: () => subtle.importKey("jwk", oct("AAAA"), "PBKDF2", false, ["sign"]),
    extractableBeforeEmptyUsages: () => subtle.importKey("raw", new Uint8Array(3), "HKDF", true, []),
    jwk: () => subtle.importKey("jwk", oct("AAAA"), "PBKDF2", false, ["deriveBits"]),
    spki: () => subtle.importKey("spki", new Uint8Array(3), "HKDF", false, ["deriveBits"]),
    extFalse: () => subtle.importKey("jwk", oct("AAAA", { ext: false }), "HKDF", true, ["deriveBits"]),
    emptyKey: () => subtle.importKey("raw", new Uint8Array(), "HKDF", false, ["deriveBits"]),
    lowerName: () => subtle.importKey("raw", new Uint8Array(3), "pbkdf2", false, ["deriveKey"]),
    export: async () => subtle.exportKey("raw", await hkdf()),
  });
  result.generate = await all({
    hmac20: async () => {
      const key = await subtle.generateKey({ ...sha256, length: 20 }, true, ["sign"]);
      const bytes = new Uint8Array(await subtle.exportKey("raw", key));
      return [describe(key), bytes.length, bytes[2] & 0x0f];
    },
    hmac1: async () => { const bytes = new Uint8Array(await subtle.exportKey("raw", await subtle.generateKey({ ...sha256, length: 1 }, true, ["sign"]))); return [bytes.length, bytes[0] & 0x7f]; },
    hmacLarge: async () => {
      const key = await subtle.generateKey({ ...sha256, length: 2 ** 20 + 3 }, true, ["sign"]);
      const bytes = new Uint8Array(await subtle.exportKey("raw", key));
      return [describe(key), bytes.length, bytes[bytes.length - 1] & 0x1f];
    },
    hmacImportLarge: () => subtle.importKey("raw", new Uint8Array(200000), sha256, true, ["sign"]),
    hmacFraction: () => subtle.generateKey({ ...sha256, length: 20.5 }, true, ["sign"]),
    hmacNegative: () => subtle.generateKey({ ...sha256, length: -8 }, true, ["sign"]),
    hmacZero: () => subtle.generateKey({ ...sha256, length: 0 }, true, ["sign"]),
    hmacNaN: () => subtle.generateKey({ ...sha256, length: "abc" }, true, ["sign"]),
    hmacHuge: () => subtle.generateKey({ name: "HMAC", hash: "SHA-999", length: 2 ** 31 }, true, ["sign"]),
    hmacSha1: () => subtle.generateKey({ name: "HMAC", hash: "sha-1" }, true, ["verify", "sign"]),
    hmacSha384: () => subtle.generateKey({ name: "HMAC", hash: "SHA-384" }, true, ["sign"]),
    hmacSha512: () => subtle.generateKey({ name: "HMAC", hash: { name: "SHA-512" } }, true, ["sign"]),
    hmacMd5: () => subtle.generateKey({ name: "HMAC", hash: "MD5" }, true, ["sign"]),
    hmacNoHash: () => subtle.generateKey({ name: "HMAC", length: 0 }, true, ["encrypt"]),
    hmacBadHash: () => subtle.generateKey({ name: "HMAC", hash: "SHA-999" }, true, ["sign"]),
    hmacJwk: async () => subtle.exportKey("jwk", await subtle.generateKey({ ...sha256, length: 24 }, true, ["verify", "sign"])).then(jwk => [jwk.key_ops, jwk.alg, jwk.k.length]),
    aesString: () => subtle.generateKey({ name: "AES-GCM", length: "128" }, true, ["encrypt"]),
    aesFraction: () => subtle.generateKey({ name: "AES-GCM", length: 128.5 }, true, ["encrypt"]),
    aes100: () => subtle.generateKey({ name: "AES-GCM", length: 100 }, true, ["encrypt"]),
    aesZero: () => subtle.generateKey({ name: "AES-GCM", length: "abc" }, true, ["encrypt"]),
    aesHuge: () => subtle.generateKey({ name: "AES-GCM", length: 2 ** 31 }, true, ["encrypt"]),
    aesNoLength: () => subtle.generateKey({ name: "AES-GCM" }, true, ["encrypt"]),
    aesLower: () => subtle.generateKey({ name: "aes-kw", length: 256 }, true, ["wrapKey"]),
    aesHashNull: () => subtle.generateKey({ name: "AES-GCM", length: 128, hash: null }, true, ["encrypt"]),
    aesExponent: () => subtle.generateKey({ name: "AES-GCM", length: 128, publicExponent: "x" }, true, ["encrypt"]),
    hkdf: () => subtle.generateKey({ name: "HKDF" }, false, ["sign"]),
    pbkdf2: () => subtle.generateKey("PBKDF2", false, ["deriveBits"]),
    unknown: () => subtle.generateKey("FOO", false, ["sign"]),
    usagesNull: () => subtle.generateKey({ name: "AES-GCM", length: 128 }, false, null),
    algorithmNull: () => subtle.generateKey(null, false, ["sign"]),
  });
  result.deriveBits = await all({
    hkdfNoSalt: hkdfBits({ salt: undefined }, 256),
    hkdfNoInfo: hkdfBits({ info: undefined }, 256),
    hkdfNoHash: hkdfBits({ hash: undefined }, 256),
    hkdfHashNull: hkdfBits({ hash: null }, 256),
    hkdfBadHash: hkdfBits({ hash: "SHA-999", salt: undefined }, 256),
    hkdfMd5: hkdfBits({ hash: "MD5" }, 128),
    hkdfSaltString: hkdfBits({ salt: "abc" }, 256),
    hkdfSaltNull: hkdfBits({ salt: null }, 256),
    hkdfInfoString: hkdfBits({ info: "abc" }, 256),
    hkdfIterations: hkdfBits({ iterations: 2 ** 40 }, 16),
    hkdfPublic: hkdfBits({ public: {} }, 16),
    hkdfLengthNull: hkdfBits({}, null),
    hkdfLengthMissing: async () => subtle.deriveBits(hkdfParams, await hkdf()),
    hkdfLengthZero: hkdfBits({}, 0),
    hkdfLengthNegative: hkdfBits({ salt: undefined }, -8),
    hkdfLength20: hkdfBits({}, 20),
    hkdfLengthHuge: hkdfBits({}, 2 ** 32),
    hkdfLengthString: hkdfBits({}, "16"),
    hkdfLengthFraction: hkdfBits({}, 16.9),
    hkdfLengthNaN: hkdfBits({}, NaN),
    hkdfTooLong: hkdfBits({}, 255 * 32 * 8 + 8),
    hkdfLowerName: async () => subtle.deriveBits({ ...hkdfParams, name: "hkdf" }, await hkdf(), 16),
    hkdfStringAlgorithm: async () => subtle.deriveBits("HKDF", await hkdf(), 16),
    hkdfEmptyKey: async () => subtle.deriveBits(hkdfParams, await subtle.importKey("raw", new Uint8Array(), "HKDF", false, ["deriveBits"]), 16),
    wrongAlgorithm: async () => subtle.deriveBits({ ...hkdfParams, name: "PBKDF2" }, await hkdf(), 16),
    unknownAlgorithm: async () => subtle.deriveBits({ ...hkdfParams, name: "FOO" }, await hkdf(), 16),
    emptyAlgorithm: async () => subtle.deriveBits({}, await hkdf(), 16),
    nullAlgorithm: async () => subtle.deriveBits(null, await hkdf(), 16),
    noUsage: async () => subtle.deriveBits(hkdfParams, await hkdf(["deriveKey"]), 16),
    notKey: () => subtle.deriveBits(hkdfParams, {}, -8),
    hmacKey: async () => subtle.deriveBits({ ...hkdfParams, name: "HMAC" }, await hmacKey(), 16),
    pbkdf2NoIterations: pbkdf2Bits({ iterations: undefined }, null),
    pbkdf2NoSalt: pbkdf2Bits({ salt: undefined, iterations: undefined }, 16),
    pbkdf2BadHash: pbkdf2Bits({ hash: "SHA-999", salt: undefined }, 16),
    pbkdf2Md5: pbkdf2Bits({ hash: "MD5" }, 16),
    pbkdf2IterationsZero: pbkdf2Bits({ iterations: 0 }, 256),
    pbkdf2IterationsNegative: pbkdf2Bits({ iterations: -1 }, 256),
    pbkdf2IterationsString: pbkdf2Bits({ iterations: "3" }, 16),
    pbkdf2IterationsFraction: pbkdf2Bits({ iterations: 2.9 }, 16),
    pbkdf2IterationsNaN: pbkdf2Bits({ iterations: NaN }, 16),
    pbkdf2IterationsHuge: pbkdf2Bits({ iterations: 2 ** 31 }, 16),
    pbkdf2LengthBeforeIterations: pbkdf2Bits({ iterations: 0 }, 20),
    pbkdf2LengthNull: pbkdf2Bits({}, null),
    pbkdf2LengthZero: pbkdf2Bits({}, 0),
    pbkdf2EmptyPassword: async () => subtle.deriveBits(pbkdf2Params, await subtle.importKey("raw", new Uint8Array(), "PBKDF2", false, ["deriveBits"]), 16),
  });
  result.deriveKey = await all({
    aesNoLength: hkdfKey({ name: "AES-GCM" }, ["encrypt"]),
    aesString: hkdfKey("AES-GCM", ["encrypt"]),
    aes100: hkdfKey({ name: "AES-GCM", length: 100 }, ["encrypt"]),
    aesLengthString: hkdfKey({ name: "AES-GCM", length: "128" }, ["encrypt"]),
    aesLowerName: hkdfKey({ name: "aes-gcm", length: 128 }, ["encrypt"]),
    aesBadUsage: hkdfKey({ name: "AES-GCM", length: 128 }, ["sign"]),
    lengthBeforeUsages: hkdfKey({ name: "AES-GCM", length: 100 }, ["sign"]),
    kwZero: hkdfKey({ name: "AES-KW", length: 0 }, ["wrapKey"]),
    huge: hkdfKey({ name: "AES-GCM", length: 2 ** 40 }, ["encrypt"]),
    nullAlgorithm: hkdfKey(null),
    hmacZero: hkdfKey({ ...sha256, length: 0 }),
    hmacNegative: hkdfKey({ ...sha256, length: -8 }),
    hmac20: hkdfKey({ ...sha256, length: 20 }),
    hmacNoHash: hkdfKey({ name: "HMAC", length: 16 }),
    hmacNoHashOrLength: hkdfKey({ name: "HMAC" }),
    hmacString: hkdfKey("HMAC"),
    hmacBadHash: hkdfKey({ name: "HMAC", hash: "SHA-999" }),
    hmacMd5: hkdfKey({ name: "HMAC", hash: "MD5" }),
    hmacSha1: hkdfKey({ name: "HMAC", hash: "SHA-1" }),
    hmacSha384: hkdfKey({ name: "HMAC", hash: "SHA-384" }),
    hmacLowerName: hkdfKey({ name: "hmac", hash: "sha-256", length: 16 }),
    hmacBytes: async () => hex(await subtle.exportKey("raw", await hkdfKey({ ...sha256, length: 16 })())),
    hkdf: async () => subtle.deriveKey(hkdfParams, await hkdf(), "HKDF", false, ["deriveBits"]),
    pbkdf2: async () => subtle.deriveKey(hkdfParams, await hkdf(), "PBKDF2", false, ["deriveBits"]),
    ecdsa: hkdfKey({ name: "ECDSA", namedCurve: "P-256" }),
    unknown: hkdfKey("FOO"),
    noUsage: async () => subtle.deriveKey(hkdfParams, await hkdf(["deriveBits"]), { name: "AES-GCM", length: 100 }, true, ["encrypt"]),
    derivedBeforeParams: async () => subtle.deriveKey({ ...hkdfParams, salt: undefined }, await hkdf(), { name: "AES-GCM", length: 100 }, true, ["encrypt"]),
    params: async () => subtle.deriveKey({ ...hkdfParams, salt: undefined }, await hkdf(), { name: "AES-GCM", length: 128 }, true, ["encrypt"]),
    wrongAlgorithm: async () => subtle.deriveKey({ ...hkdfParams, name: "PBKDF2" }, await hkdf(), { name: "AES-GCM", length: 128 }, true, ["encrypt"]),
    pbkdf2Hmac20: async () => subtle.deriveKey(pbkdf2Params, await pbkdf2(), { ...sha256, length: 20 }, true, ["sign"]),
    pbkdf2Aes: async () => subtle.deriveKey(pbkdf2Params, await pbkdf2(), { name: "AES-CBC", length: 256 }, true, ["decrypt", "encrypt"]),
    usagesNull: async () => subtle.deriveKey(hkdfParams, await hkdf(), { name: "AES-GCM", length: 128 }, true, null),
  });
  result.export = await all({
    hmacJwk: async () => { const jwk = await subtle.exportKey("jwk", await hmacKey(["verify", "sign"])); return [Object.keys(jwk), jwk]; },
    aesJwk: async () => { const jwk = await subtle.exportKey("jwk", await aesKey("AES-GCM", ["unwrapKey", "decrypt"])); return [Object.keys(jwk), jwk]; },
    kwJwk: async () => subtle.exportKey("jwk", await aesKey("AES-KW", ["wrapKey", "unwrapKey"])),
    ctrJwk: async () => subtle.exportKey("jwk", await subtle.importKey("raw", new Uint8Array(32), "AES-CTR", true, ["encrypt"])),
    ecJwkFields: async () => Object.keys(await subtle.exportKey("jwk", (await subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign"])).privateKey)),
    upperRaw: async () => subtle.exportKey("RAW", await hmacKey()),
    hmacSpki: async () => subtle.exportKey("spki", await hmacKey()),
    aesSpki: async () => subtle.exportKey("spki", await aesKey("AES-GCM")),
    aesNumber: async () => subtle.exportKey(5, await aesKey("AES-GCM")),
    extractableBeforeFormat: async () => subtle.exportKey("foo", await aesKey("AES-GCM", ["encrypt"], false)),
    notKey: () => subtle.exportKey("raw", {}),
  });
  result.wrap = await all({
    jwkText: async () => { const key = await aesKey("AES-GCM"); return new TextDecoder().decode(await subtle.decrypt(gcm, key, await subtle.wrapKey("jwk", await hmacKey(["verify", "sign"]), key, gcm))); },
    kwLowerName: async () => hex(await subtle.wrapKey("raw", await aesKey("AES-GCM"), await aesKey("AES-KW"), "aes-kw")),
    kwShort: async () => subtle.wrapKey("raw", await hmacKey(), await aesKey("AES-KW"), "AES-KW"),
    cbc: async () => hex(await subtle.wrapKey("raw", await hmacKey(), await aesKey("AES-CBC"), { name: "AES-CBC", iv: new Uint8Array(16) })),
    ctr: async () => hex(await subtle.wrapKey("raw", await hmacKey(), await aesKey("AES-CTR"), ctr)),
    wrongAlgorithm: async () => subtle.wrapKey("raw", await aesKey("AES-GCM"), await aesKey("AES-KW"), gcm),
    unknownAlgorithm: async () => subtle.wrapKey("raw", await aesKey("AES-GCM"), await aesKey("AES-KW"), "FOO"),
    noUsage: async () => subtle.wrapKey("raw", await aesKey("AES-GCM"), await aesKey("AES-KW", ["unwrapKey"]), "AES-KW"),
    hmacWrapping: async () => subtle.wrapKey("raw", await aesKey("AES-GCM"), await hmacKey(), "HMAC"),
    notExtractable: async () => subtle.wrapKey("foo", await aesKey("AES-GCM", ["encrypt"], false), await aesKey("AES-KW"), "AES-KW"),
    badFormat: async () => subtle.wrapKey("RAW", await aesKey("AES-GCM"), await aesKey("AES-KW"), "AES-KW"),
    notKey: async () => subtle.wrapKey("raw", {}, await aesKey("AES-KW"), "AES-KW"),
    gcmNoIv: async () => subtle.wrapKey("raw", await hmacKey(), await aesKey("AES-GCM"), "AES-GCM"),
    ctrNoLength: async () => subtle.wrapKey("raw", await hmacKey(), await aesKey("AES-CTR"), { name: "AES-CTR", counter: new Uint8Array(16) }),
    kwIvString: async () => subtle.wrapKey("raw", await aesKey("AES-GCM"), await aesKey("AES-KW"), { name: "AES-KW", iv: "x" }),
    unwrapJson: gcmUnwrap(text("{"), sha256),
    unwrapBom: gcmUnwrap(text('﻿{"kty":"oct","k":"AAAA"}'), sha256),
    unwrapNoKeyType: gcmUnwrap(text('{"k":"AAAA"}'), sha256),
    unwrapArray: gcmUnwrap(text("[]"), sha256),
    unwrapNull: gcmUnwrap(text("null"), sha256),
    unwrapNumber: gcmUnwrap(text("5"), sha256),
    unwrapKeyOpsString: gcmUnwrap(text('{"kty":"oct","k":"AAAA","key_ops":"x"}'), sha256),
    unwrapJwk: gcmUnwrap(text('{"kty":"oct","k":"AAAA","key_ops":["sign","sign"]}'), sha256, ["verify", "sign"]),
    unwrapExtFalse: gcmUnwrap(text('{"kty":"oct","k":"AAAA","ext":false}'), sha256),
    unwrapRaw: gcmUnwrap(text("abc"), sha256, ["sign"], "raw"),
    unwrapBadFormat: gcmUnwrap(text("abc"), sha256, ["sign"], "foo"),
    unwrapBadAlgorithm: gcmUnwrap(text("abc"), "FOO", ["sign"], "raw"),
    unwrapHkdfExtractable: gcmUnwrap(text("abc"), "HKDF", ["deriveBits"], "raw"),
    unwrapWrongAlgorithm: async () => subtle.unwrapKey("raw", new Uint8Array(24), await aesKey("AES-GCM"), "AES-KW", "AES-GCM", true, ["encrypt"]),
    unwrapNoUsage: async () => subtle.unwrapKey("raw", new Uint8Array(24), await aesKey("AES-KW", ["wrapKey"]), "AES-KW", "AES-GCM", true, ["encrypt"]),
    unwrapCorrupt: async () => subtle.unwrapKey("raw", new Uint8Array(24), await aesKey("AES-KW"), "AES-KW", "AES-GCM", true, ["encrypt"]),
    unwrapShort: async () => subtle.unwrapKey("raw", new Uint8Array(16), await aesKey("AES-KW"), "AES-KW", "AES-GCM", true, ["encrypt"]),
    unwrapNotBytes: async () => subtle.unwrapKey("raw", "abc", await aesKey("AES-KW"), "AES-KW", "AES-GCM", true, ["encrypt"]),
    roundTrip: async () => {
      const kek = await aesKey("AES-KW");
      const wrapped = await subtle.wrapKey("raw", await aesKey("AES-GCM"), kek, "AES-KW");
      return Promise.all([["decrypt", "encrypt"], ["sign"]].map(usages => attempt(() => subtle.unwrapKey("raw", wrapped, kek, "AES-KW", "AES-GCM", true, usages))));
    },
  });
  result.operations = await all({
    signWrongAlgorithm: async () => subtle.sign("AES-GCM", await hmacKey(), new Uint8Array(1)),
    signEmptyAlgorithm: async () => subtle.sign({}, await hmacKey(), new Uint8Array(1)),
    signNullAlgorithm: async () => subtle.sign(null, await hmacKey(), new Uint8Array(1)),
    signNoUsage: async () => subtle.sign("HMAC", await hmacKey(["verify"]), new Uint8Array(1)),
    signNotKey: () => subtle.sign("HMAC", {}, new Uint8Array(1)),
    signLowerName: async () => hex(await subtle.sign({ name: "hmac" }, await hmacKey(), new Uint8Array(1))),
    verifyNotBytes: async () => subtle.verify("HMAC", await hmacKey(), "x", new Uint8Array(1)),
    verifyLength20: async () => {
      const key = await subtle.importKey("raw", ones(), { name: "HMAC", hash: "SHA-1", length: 20 }, false, ["sign", "verify"]);
      const signature = await subtle.sign("HMAC", key, new Uint8Array([1]));
      return [hex(signature), await subtle.verify("HMAC", key, signature, new Uint8Array([1]))];
    },
    encryptWrongAlgorithm: async () => subtle.encrypt({ name: "AES-CBC", iv: new Uint8Array(16) }, await aesKey("AES-GCM"), new Uint8Array(1)),
    encryptNoUsage: async () => subtle.encrypt(gcm, await aesKey("AES-GCM", ["decrypt"]), new Uint8Array(1)),
    encryptKw: async () => subtle.encrypt("AES-KW", await aesKey("AES-KW"), new Uint8Array(16)),
    gcmIvString: async () => subtle.encrypt({ name: "AES-GCM", iv: "abc" }, await aesKey("AES-GCM"), new Uint8Array(1)),
    gcmAdditionalDataNull: async () => subtle.encrypt({ ...gcm, additionalData: null }, await aesKey("AES-GCM"), new Uint8Array(1)),
    gcmTagHuge: async () => subtle.encrypt({ ...gcm, tagLength: 2 ** 40 }, await aesKey("AES-GCM"), new Uint8Array(1)),
    gcmTagString: async () => hex(await subtle.encrypt({ ...gcm, tagLength: "96" }, await aesKey("AES-GCM"), new Uint8Array(1))),
    gcmTagNaN: async () => subtle.encrypt({ ...gcm, tagLength: "abc" }, await aesKey("AES-GCM"), new Uint8Array(1)),
    ctrNoCounter: async () => subtle.encrypt({ name: "AES-CTR", length: 64 }, await aesKey("AES-CTR"), new Uint8Array(1)),
    ctrLengthZero: async () => subtle.encrypt({ ...ctr, length: 0 }, await aesKey("AES-CTR"), new Uint8Array(1)),
    cbcNoIv: async () => subtle.encrypt({ name: "AES-CBC" }, await aesKey("AES-CBC"), new Uint8Array(1)),
  });
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
    hkdfTooLong: (() => { try { return nodeCrypto.hkdfSync("sha256", "key", "salt", "info", 255 * 32 + 1); } catch (error) { return [error.name, error.message]; } })(),
  };
  const ecPair = nodeCrypto.generateKeyPairSync("ec", { namedCurve: "P-256" });
  const details = keyObject => {
    const value = keyObject.asymmetricKeyDetails;
    return value && Object.fromEntries(Object.entries(value).map(([name, entry]) => [name, String(entry)]));
  };
  result.keyDetails = [privateKey, publicKey, ecPair.publicKey, nodeCrypto.generateKeyPairSync("ed25519").privateKey, nodeCrypto.createSecretKey(Buffer.from([1]))].map(keyObject => outcome(() => details(keyObject)));
  result.generatedPairs = {
    ec: nodeCrypto.verify("sha256", Buffer.from("abc"), ecPair.publicKey, nodeCrypto.sign("sha256", Buffer.from("abc"), ecPair.privateKey)),
    ecJwkFields: definedKeys(ecPair.privateKey.export({ format: "jwk" })),
    rsa: (() => { const pair = nodeCrypto.generateKeyPairSync("rsa", { modulusLength: 1024 }); return nodeCrypto.verify("sha256", Buffer.from("abc"), pair.publicKey, nodeCrypto.sign("sha256", Buffer.from("abc"), pair.privateKey)); })(),
    ed25519: (() => { const pair = nodeCrypto.generateKeyPairSync("ed25519"); return nodeCrypto.verify(null, Buffer.from("abc"), pair.publicKey, nodeCrypto.sign(null, Buffer.from("abc"), pair.privateKey)); })(),
  };
  return result;
}
