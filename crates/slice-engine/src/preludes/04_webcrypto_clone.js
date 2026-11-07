// Slice (Nexus-JS) Lazy Prelude Shard 4: WebCrypto (`crypto.subtle`, `CryptoKey`, `crypto.randomUUID`, `crypto.getRandomValues`) (`04_webcrypto_clone.js`)
(function() {
  function __algName(alg) {
    if (typeof alg === "string") return alg.trim().toUpperCase();
    if (alg && typeof alg === "object" && typeof alg.name === "string") return alg.name.trim().toUpperCase();
    return "";
  }

  function __normHashName(h) {
    var s = "";
    if (typeof h === "string") s = h.trim().toUpperCase();
    else if (h && typeof h === "object" && typeof h.name === "string") s = h.name.trim().toUpperCase();
    if (s === "SHA256" || s === "SHA-256") return "SHA-256";
    if (s === "SHA384" || s === "SHA-384") return "SHA-384";
    if (s === "SHA512" || s === "SHA-512") return "SHA-512";
    if (s === "SHA1" || s === "SHA-1") return "SHA-1";
    return "SHA-256";
  }

  function __hmacHashName(alg) {
    if (!alg || typeof alg !== "object") return "SHA-256";
    return __normHashName(alg.hash);
  }

  function __curveName(alg) {
    if (!alg || typeof alg !== "object") return "P-256";
    var c = String(alg.namedCurve || "P-256").trim().toUpperCase();
    if (c === "P-384" || c === "SECP384R1") return "P-384";
    return "P-256";
  }

  function CryptoKey(type, extractable, algorithm, usages, rawU8, handleId, meta) {
    this.type = type;
    this.extractable = Boolean(extractable);
    this.algorithm = algorithm;
    this.usages = Array.isArray(usages) ? usages.slice() : [];
    this._raw = rawU8 || null;
    this._id = Number(handleId) || 0;
    this._meta = meta || null;
  }
  globalThis.CryptoKey = CryptoKey;

  var __subtle = {
    digest: function(algorithm, data) {
      try {
        var name = __algName(algorithm);
        var ab = __slice_crypto_digest(name, data);
        return Promise.resolve(ab);
      } catch (e) {
        return Promise.reject(e);
      }
    },
    generateKey: function(algorithm, extractable, keyUsages) {
      try {
        var name = __algName(algorithm);
        var usages = Array.isArray(keyUsages) ? keyUsages.slice() : [];
        if (name === "AES-GCM" || name === "AES-CBC") {
          var bitLen = Number(algorithm && algorithm.length) || 256;
          if (bitLen !== 128 && bitLen !== 256) {
            throw new Error("OperationError: AES key length must be 128 or 256 bits");
          }
          var rawBytes = __slice_crypto_random_bytes(bitLen / 8);
          var hid = __slice_crypto_key_register(name, "", rawBytes);
          return Promise.resolve(new CryptoKey("secret", extractable, { name: name, length: bitLen }, usages, rawBytes, hid, null));
        }
        if (name === "HMAC") {
          var hName = __hmacHashName(algorithm);
          var defaultBits = hName === "SHA-512" ? 512 : (hName === "SHA-384" ? 384 : 256);
          var hBits = Number(algorithm && algorithm.length) || defaultBits;
          var hBytes = __slice_crypto_random_bytes(Math.ceil(hBits / 8));
          var hId = __slice_crypto_key_register("HMAC", hName, hBytes);
          return Promise.resolve(new CryptoKey("secret", extractable, { name: "HMAC", hash: { name: hName }, length: hBytes.byteLength * 8 }, usages, hBytes, hId, null));
        }
        if (name === "ED25519") {
          var edInfo = JSON.parse(__slice_crypto_asym_keygen("Ed25519", ""));
          var pubUsages = usages.filter(function(u) { return u === "verify"; });
          var privUsages = usages.filter(function(u) { return u === "sign"; });
          var pubRaw = __slice_base64_decode_bytes(edInfo.pubRaw);
          var pubKey = new CryptoKey("public", true, { name: "Ed25519" }, pubUsages, pubRaw, edInfo.pubId, edInfo);
          var privKey = new CryptoKey("private", extractable, { name: "Ed25519" }, privUsages, __slice_base64_decode_bytes(edInfo.pkcs8), edInfo.privId, edInfo);
          return Promise.resolve({ publicKey: pubKey, privateKey: privKey });
        }
        if (name === "ECDSA") {
          var crv = __curveName(algorithm);
          var ecInfo = JSON.parse(__slice_crypto_asym_keygen("ECDSA", crv));
          var ecPubUsages = usages.filter(function(u) { return u === "verify"; });
          var ecPrivUsages = usages.filter(function(u) { return u === "sign"; });
          var ecPubRaw = __slice_base64_decode_bytes(ecInfo.pubRaw);
          var ecPubKey = new CryptoKey("public", true, { name: "ECDSA", namedCurve: crv }, ecPubUsages, ecPubRaw, ecInfo.pubId, ecInfo);
          var ecPrivKey = new CryptoKey("private", extractable, { name: "ECDSA", namedCurve: crv }, ecPrivUsages, __slice_base64_decode_bytes(ecInfo.pkcs8), ecInfo.privId, ecInfo);
          return Promise.resolve({ publicKey: ecPubKey, privateKey: ecPrivKey });
        }
        throw new Error("NotSupportedError: Unsupported generateKey algorithm '" + name + "'");
      } catch (e) {
        return Promise.reject(e);
      }
    },
    importKey: function(format, keyData, algorithm, extractable, keyUsages) {
      try {
        var fmt = String(format).toLowerCase();
        var name = __algName(algorithm);
        var usages = Array.isArray(keyUsages) ? keyUsages.slice() : [];
        if (name === "HMAC") {
          var hashName = __hmacHashName(algorithm);
          var rawU8;
          if (fmt === "raw") {
            rawU8 = new Uint8Array(__slice_toU8(keyData));
          } else if (fmt === "jwk") {
            if (!keyData || typeof keyData.k !== "string") {
              throw new TypeError("DataError: Invalid JWK keyData for HMAC");
            }
            rawU8 = __slice_base64_decode_bytes(keyData.k);
          } else {
            throw new Error("NotSupportedError: Unsupported key format '" + fmt + "'");
          }
          if (rawU8.byteLength === 0) {
            throw new TypeError("DataError: HMAC key length cannot be zero");
          }
          var hId = __slice_crypto_key_register("HMAC", hashName, rawU8);
          var keyAlg = { name: "HMAC", hash: { name: hashName }, length: rawU8.byteLength * 8 };
          return Promise.resolve(new CryptoKey("secret", extractable, keyAlg, usages, rawU8, hId, null));
        }
        if (name === "AES-GCM" || name === "AES-CBC") {
          var aesU8;
          if (fmt === "raw") {
            aesU8 = new Uint8Array(__slice_toU8(keyData));
          } else if (fmt === "jwk") {
            if (!keyData || typeof keyData.k !== "string") {
              throw new TypeError("DataError: Invalid JWK keyData for " + name);
            }
            aesU8 = __slice_base64_decode_bytes(keyData.k);
          } else {
            throw new Error("NotSupportedError: Unsupported key format '" + fmt + "' for " + name);
          }
          if (aesU8.byteLength !== 16 && aesU8.byteLength !== 32) {
            throw new Error("DataError: AES key length must be 16 or 32 bytes");
          }
          var aesId = __slice_crypto_key_register(name, "", aesU8);
          return Promise.resolve(new CryptoKey("secret", extractable, { name: name, length: aesU8.byteLength * 8 }, usages, aesU8, aesId, null));
        }
        if (name === "PBKDF2" || name === "HKDF") {
          if (fmt !== "raw") {
            throw new Error("NotSupportedError: " + name + " only supports 'raw' importKey format");
          }
          var kdfU8 = new Uint8Array(__slice_toU8(keyData));
          return Promise.resolve(new CryptoKey("secret", false, { name: name }, usages, kdfU8, 0, null));
        }
        if (name === "ED25519") {
          var edPayload = fmt === "jwk" ? JSON.stringify(keyData) : __slice_toU8(keyData);
          var edImp = JSON.parse(__slice_crypto_asym_import("Ed25519", "", fmt, edPayload));
          var edRaw = edImp.type === "private" ? __slice_base64_decode_bytes(edImp.pkcs8) : __slice_base64_decode_bytes(edImp.pubRaw);
          return Promise.resolve(new CryptoKey(edImp.type, extractable, { name: "Ed25519" }, usages, edRaw, edImp.id, edImp));
        }
        if (name === "ECDSA") {
          var crv = __curveName(algorithm);
          var ecPayload = fmt === "jwk" ? JSON.stringify(keyData) : __slice_toU8(keyData);
          var ecImp = JSON.parse(__slice_crypto_asym_import("ECDSA", crv, fmt, ecPayload));
          var ecRaw = ecImp.type === "private" ? __slice_base64_decode_bytes(ecImp.pkcs8) : __slice_base64_decode_bytes(ecImp.pubRaw);
          return Promise.resolve(new CryptoKey(ecImp.type, extractable, { name: "ECDSA", namedCurve: crv }, usages, ecRaw, ecImp.id, ecImp));
        }
        throw new Error("NotSupportedError: Unsupported importKey algorithm '" + name + "'");
      } catch (e) {
        return Promise.reject(e);
      }
    },
    exportKey: function(format, key) {
      try {
        if (!key || !(key instanceof CryptoKey)) throw new TypeError("Invalid CryptoKey");
        if (!key.extractable) throw new Error("InvalidAccessError: key is not extractable");
        var fmt = String(format).toLowerCase();
        var kAlg = (key.algorithm && key.algorithm.name) || "";
        var kUpper = kAlg.toUpperCase();
        if (fmt === "raw") {
          if (key.type === "private") {
            throw new Error("InvalidAccessError: Cannot export private key in 'raw' format");
          }
          var copy = new Uint8Array(key._raw);
          return Promise.resolve(copy.buffer);
        }
        if (fmt === "pkcs8") {
          if (key.type !== "private" || !key._meta || !key._meta.pkcs8) {
            throw new Error("InvalidAccessError: Cannot export key in 'pkcs8' format");
          }
          var p8 = __slice_base64_decode_bytes(key._meta.pkcs8);
          return Promise.resolve(new Uint8Array(p8).buffer);
        }
        if (fmt === "spki") {
          if (key.type !== "public" || !key._meta || !key._meta.spki) {
            throw new Error("InvalidAccessError: Cannot export key in 'spki' format");
          }
          var spki = __slice_base64_decode_bytes(key._meta.spki);
          return Promise.resolve(new Uint8Array(spki).buffer);
        }
        if (fmt === "jwk") {
          if (kUpper === "HMAC") {
            var hName = (key.algorithm && key.algorithm.hash && key.algorithm.hash.name) || "SHA-256";
            var jwkAlg = hName === "SHA-512"
              ? "HS512"
              : (hName === "SHA-384" ? "HS384" : (hName === "SHA-1" ? "HS1" : "HS256"));
            return Promise.resolve({
              kty: "oct",
              k: __slice_base64_encode_bytes(key._raw, true),
              alg: jwkAlg,
              ext: Boolean(key.extractable),
              key_ops: key.usages.slice()
            });
          }
          if (kUpper === "AES-GCM" || kUpper === "AES-CBC") {
            var bits = key._raw.byteLength * 8;
            var modeSuffix = kUpper === "AES-GCM" ? "GCM" : "CBC";
            return Promise.resolve({
              kty: "oct",
              k: __slice_base64_encode_bytes(key._raw, true),
              alg: "A" + bits + modeSuffix,
              ext: Boolean(key.extractable),
              key_ops: key.usages.slice()
            });
          }
          if (kUpper === "ED25519" && key._meta) {
            var edJwk = {
              kty: "OKP",
              crv: "Ed25519",
              x: key._meta.x,
              ext: Boolean(key.extractable),
              key_ops: key.usages.slice()
            };
            if (key.type === "private" && key._meta.d) edJwk.d = key._meta.d;
            return Promise.resolve(edJwk);
          }
          if (kUpper === "ECDSA" && key._meta) {
            var ecJwk = {
              kty: "EC",
              crv: key._meta.crv || "P-256",
              x: key._meta.x,
              y: key._meta.y,
              ext: Boolean(key.extractable),
              key_ops: key.usages.slice()
            };
            if (key.type === "private" && key._meta.d) ecJwk.d = key._meta.d;
            return Promise.resolve(ecJwk);
          }
        }
        throw new Error("NotSupportedError: Unsupported exportKey format '" + fmt + "'");
      } catch (e) {
        return Promise.reject(e);
      }
    },
    encrypt: function(algorithm, key, data) {
      try {
        if (!key || !(key instanceof CryptoKey)) throw new TypeError("Invalid CryptoKey");
        if (key.usages.indexOf("encrypt") === -1) {
          throw new Error("InvalidAccessError: key does not support 'encrypt' usage");
        }
        var name = __algName(algorithm);
        var iv = algorithm && algorithm.iv;
        if (!iv) throw new TypeError("OperationError: Missing iv in encrypt algorithm");
        var aad = (algorithm && algorithm.additionalData) ? algorithm.additionalData : null;
        var tagLen = (algorithm && algorithm.tagLength) ? Number(algorithm.tagLength) : 128;
        var ab = __slice_crypto_cipher(name, true, key._id, key._raw, iv, aad, tagLen, data, true);
        return Promise.resolve(ab);
      } catch (e) {
        return Promise.reject(e);
      }
    },
    decrypt: function(algorithm, key, data) {
      try {
        if (!key || !(key instanceof CryptoKey)) throw new TypeError("Invalid CryptoKey");
        if (key.usages.indexOf("decrypt") === -1) {
          throw new Error("InvalidAccessError: key does not support 'decrypt' usage");
        }
        var name = __algName(algorithm);
        var iv = algorithm && algorithm.iv;
        if (!iv) throw new TypeError("OperationError: Missing iv in decrypt algorithm");
        var aad = (algorithm && algorithm.additionalData) ? algorithm.additionalData : null;
        var tagLen = (algorithm && algorithm.tagLength) ? Number(algorithm.tagLength) : 128;
        var ab = __slice_crypto_cipher(name, false, key._id, key._raw, iv, aad, tagLen, data, true);
        return Promise.resolve(ab);
      } catch (e) {
        return Promise.reject(e);
      }
    },
    sign: function(algorithm, key, data) {
      try {
        if (!key || !(key instanceof CryptoKey)) throw new TypeError("Invalid CryptoKey");
        if (key.usages.indexOf("sign") === -1) {
          throw new Error("InvalidAccessError: key does not support 'sign' usage");
        }
        var name = __algName(algorithm) || __algName(key.algorithm);
        if (name === "ED25519") {
          return Promise.resolve(__slice_crypto_asym_sign("Ed25519", "", key._id, key._raw, data, false));
        }
        if (name === "ECDSA") {
          var crv = (key.algorithm && key.algorithm.namedCurve) || "P-256";
          return Promise.resolve(__slice_crypto_asym_sign("ECDSA", crv, key._id, key._raw, data, false));
        }
        var hashName = (key.algorithm && key.algorithm.hash && key.algorithm.hash.name) || __hmacHashName(algorithm);
        var ab = __slice_crypto_hmac_sign(hashName, key._raw, data, key._id);
        return Promise.resolve(ab);
      } catch (e) {
        return Promise.reject(e);
      }
    },
    verify: function(algorithm, key, signature, data) {
      try {
        if (!key || !(key instanceof CryptoKey)) throw new TypeError("Invalid CryptoKey");
        if (key.usages.indexOf("verify") === -1) {
          throw new Error("InvalidAccessError: key does not support 'verify' usage");
        }
        var name = __algName(algorithm) || __algName(key.algorithm);
        if (name === "ED25519") {
          return Promise.resolve(Boolean(__slice_crypto_asym_verify("Ed25519", "", key._id, key._raw, signature, data)));
        }
        if (name === "ECDSA") {
          var crv = (key.algorithm && key.algorithm.namedCurve) || "P-256";
          return Promise.resolve(Boolean(__slice_crypto_asym_verify("ECDSA", crv, key._id, key._raw, signature, data)));
        }
        var hashName = (key.algorithm && key.algorithm.hash && key.algorithm.hash.name) || __hmacHashName(algorithm);
        var ok = __slice_crypto_hmac_verify(hashName, key._raw, signature, data, key._id);
        return Promise.resolve(Boolean(ok));
      } catch (e) {
        return Promise.reject(e);
      }
    },
    deriveBits: function(algorithm, baseKey, length) {
      try {
        if (!baseKey || !(baseKey instanceof CryptoKey)) throw new TypeError("Invalid baseKey");
        if (baseKey.usages.indexOf("deriveBits") === -1 && baseKey.usages.indexOf("deriveKey") === -1) {
          throw new Error("InvalidAccessError: baseKey does not support deriveBits/deriveKey usage");
        }
        var name = __algName(algorithm);
        var bitLen = Number(length) || 0;
        if (bitLen <= 0 || bitLen % 8 !== 0) {
          throw new Error("OperationError: deriveBits length must be a positive multiple of 8");
        }
        var hashName = __hmacHashName(algorithm);
        var salt = (algorithm && algorithm.salt) ? algorithm.salt : new Uint8Array(0);
        if (name === "PBKDF2") {
          var iters = Number(algorithm && algorithm.iterations) || 0;
          var ab = __slice_crypto_kdf("PBKDF2", hashName, baseKey._raw, salt, null, iters, bitLen / 8);
          return Promise.resolve(ab);
        }
        if (name === "HKDF") {
          var info = (algorithm && algorithm.info) ? algorithm.info : new Uint8Array(0);
          var hkAb = __slice_crypto_kdf("HKDF", hashName, baseKey._raw, salt, info, 1, bitLen / 8);
          return Promise.resolve(hkAb);
        }
        throw new Error("NotSupportedError: Unsupported deriveBits algorithm '" + name + "'");
      } catch (e) {
        return Promise.reject(e);
      }
    },
    deriveKey: function(algorithm, baseKey, derivedKeyType, extractable, keyUsages) {
      var self = this;
      try {
        if (!baseKey || !(baseKey instanceof CryptoKey)) throw new TypeError("Invalid baseKey");
        if (baseKey.usages.indexOf("deriveKey") === -1) {
          throw new Error("InvalidAccessError: baseKey does not support 'deriveKey' usage");
        }
        var dName = __algName(derivedKeyType);
        var bitLen = Number(derivedKeyType && derivedKeyType.length) || 256;
        if (dName === "HMAC" && !(derivedKeyType && derivedKeyType.length)) {
          var dh = __hmacHashName(derivedKeyType);
          bitLen = dh === "SHA-512" ? 512 : (dh === "SHA-384" ? 384 : 256);
        }
        return self.deriveBits(algorithm, baseKey, bitLen).then(function(ab) {
          return self.importKey("raw", ab, derivedKeyType, extractable, keyUsages);
        });
      } catch (e) {
        return Promise.reject(e);
      }
    },
    timingSafeEqual: function(a, b) {
      return __slice_crypto_timing_safe_equal(a, b);
    }
  };

  globalThis.crypto = {
    randomUUID: function() {
      return __slice_crypto_random_uuid();
    },
    getRandomValues: function(typedArray) {
      return __slice_crypto_get_random_values(typedArray);
    },
    subtle: __subtle
  };
})();
