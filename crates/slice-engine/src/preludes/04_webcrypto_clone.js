// Slice (Nexus-JS) Lazy Prelude Shard 4: WebCrypto (`crypto.subtle`, `CryptoKey`, `crypto.randomUUID`, `crypto.getRandomValues`) (`04_webcrypto_clone.js`)
(function() {
  function __algName(alg) {
    if (typeof alg === "string") return alg.trim().toUpperCase();
    if (alg && typeof alg === "object" && typeof alg.name === "string") return alg.name.trim().toUpperCase();
    return "";
  }

  function __hmacHashName(alg) {
    if (!alg || typeof alg !== "object") return "SHA-256";
    var h = alg.hash;
    if (typeof h === "string") return h.trim().toUpperCase();
    if (h && typeof h === "object" && typeof h.name === "string") return h.name.trim().toUpperCase();
    return "SHA-256";
  }

  function CryptoKey(type, extractable, algorithm, usages, rawU8) {
    this.type = type;
    this.extractable = Boolean(extractable);
    this.algorithm = algorithm;
    this.usages = Array.isArray(usages) ? usages.slice() : [];
    this._raw = rawU8;
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
    importKey: function(format, keyData, algorithm, extractable, keyUsages) {
      try {
        var fmt = String(format).toLowerCase();
        var name = __algName(algorithm);
        if (name !== "HMAC") {
          throw new Error("NotSupportedError: Unsupported importKey algorithm '" + name + "'");
        }
        var hashName = __hmacHashName(algorithm);
        if (hashName === "SHA256") hashName = "SHA-256";
        else if (hashName === "SHA384") hashName = "SHA-384";
        else if (hashName === "SHA512") hashName = "SHA-512";
        else if (hashName === "SHA1") hashName = "SHA-1";
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
        var keyAlg = { name: "HMAC", hash: { name: hashName }, length: rawU8.byteLength * 8 };
        return Promise.resolve(new CryptoKey("secret", extractable, keyAlg, keyUsages, rawU8));
      } catch (e) {
        return Promise.reject(e);
      }
    },
    exportKey: function(format, key) {
      try {
        if (!key || !(key instanceof CryptoKey)) throw new TypeError("Invalid CryptoKey");
        if (!key.extractable) throw new Error("InvalidAccessError: key is not extractable");
        var fmt = String(format).toLowerCase();
        if (fmt === "raw") {
          var copy = new Uint8Array(key._raw);
          return Promise.resolve(copy.buffer);
        }
        if (fmt === "jwk") {
          var hName = (key.algorithm && key.algorithm.hash && key.algorithm.hash.name) || "SHA-256";
          var jwkAlg = hName === "SHA-512"
            ? "HS512"
            : (hName === "SHA-384" ? "HS384" : (hName === "SHA-1" ? "HS1" : "HS256"));
          return Promise.resolve({
            kty: "oct",
            k: __slice_base64_encode_bytes(key._raw, true),
            alg: jwkAlg,
            ext: true,
            key_ops: key.usages.slice()
          });
        }
        throw new Error("NotSupportedError: Unsupported exportKey format '" + fmt + "'");
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
        var hashName = (key.algorithm && key.algorithm.hash && key.algorithm.hash.name) || __hmacHashName(algorithm);
        var ab = __slice_crypto_hmac_sign(hashName, key._raw, data);
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
        var hashName = (key.algorithm && key.algorithm.hash && key.algorithm.hash.name) || __hmacHashName(algorithm);
        var ok = __slice_crypto_hmac_verify(hashName, key._raw, signature, data);
        return Promise.resolve(Boolean(ok));
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
