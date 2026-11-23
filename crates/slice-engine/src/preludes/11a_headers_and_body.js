// Slice (Nexus-JS) Lazy Prelude Shard 11: WinterTC `Headers`, `Request`, `Response` (`11_fetch_headers_req_res.js`)
(function() {
  var __HDR_TCHAR = [
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,1,0,1,1,1,1,1,0,0,1,1,0,1,1,0,1,1,1,1,1,1,1,1,1,1,0,0,0,0,0,0,
    0,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0,0,0,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,0,1,0,1
  ];
  var __hdrNameLc = Object.create(null);
  var __hdrNameCount = 0;

  function __lcHdrName(n) {
    var s = typeof n === "string" ? n : String(n);
    var lc = __hdrNameLc[s];
    if (lc !== undefined) return lc;
    var len = s.length;
    if (len === 0) throw new TypeError("Headers: invalid empty header name");
    for (var i = 0; i < len; i++) {
      var c = s.charCodeAt(i);
      if (c > 126 || !__HDR_TCHAR[c]) {
        throw new TypeError("Headers: invalid character in header name '" + s + "'");
      }
    }
    lc = s.toLowerCase();
    if (__hdrNameCount < 512) {
      __hdrNameLc[s] = lc;
      if (lc !== s) __hdrNameLc[lc] = lc;
      __hdrNameCount++;
    }
    return lc;
  }

  var __hasOwn = Object.prototype.hasOwnProperty;
  var __hdrValCache = Object.create(null);
  var __hdrValCount = 0;

  function __normHdrVal(v) {
    if (typeof v === "string") {
      var len = v.length;
      if (len === 0) return "";
      if (len <= 96) {
        var cached = __hdrValCache[v];
        if (cached !== undefined) return cached;
      }
      for (var i = 0; i < len; i++) {
        var c = v.charCodeAt(i);
        if (c === 0 || c === 10 || c === 13) {
          throw new TypeError("Headers: invalid control character in header value");
        }
      }
      var res = v;
      var f = v.charCodeAt(0), l = v.charCodeAt(len - 1);
      if (f === 32 || f === 9 || l === 32 || l === 9) {
        var st = 0, en = len;
        while (st < en && (v.charCodeAt(st) === 32 || v.charCodeAt(st) === 9)) st++;
        while (en > st && (v.charCodeAt(en - 1) === 32 || v.charCodeAt(en - 1) === 9)) en--;
        res = v.slice(st, en);
      }
      if (len <= 96 && __hdrValCount < 512) {
        __hdrValCache[v] = res;
        __hdrValCount++;
      }
      return res;
    }
    var s = String(v);
    var sLen = s.length;
    if (sLen === 0) return "";
    for (var j = 0; j < sLen; j++) {
      var sc0 = s.charCodeAt(j);
      if (sc0 === 0 || sc0 === 10 || sc0 === 13) {
        throw new TypeError("Headers: invalid control character in header value");
      }
    }
    var sf = s.charCodeAt(0), sl = s.charCodeAt(sLen - 1);
    if (sf === 32 || sf === 9 || sl === 32 || sl === 9) {
      var sst = 0, sen = sLen;
      while (sst < sen && (s.charCodeAt(sst) === 32 || s.charCodeAt(sst) === 9)) sst++;
      while (sen > sst && (s.charCodeAt(sen - 1) === 32 || s.charCodeAt(sen - 1) === 9)) sen--;
      return s.slice(sst, sen);
    }
    return s;
  }

  function __assignHeadersMap(target, mField, scField, oh) {
    var m = Object.create(null);
    if (oh._isHeaders) {
      if (oh._live) oh._ensure();
      for (var hk in oh._m) m[hk] = oh._m[hk];
      target[mField] = m;
      target[scField] = oh._sc ? oh._sc.slice() : null;
      return;
    }
    var sc = null;
    var plainProto = Object.getPrototypeOf(oh) === Object.prototype;
    for (var key in oh) {
      if (plainProto || __hasOwn.call(oh, key)) {
        var val = oh[key];
        if (val !== undefined) {
          var lk = __lcHdrName(key), sv = __normHdrVal(val);
          m[lk] = m[lk] !== undefined ? m[lk] + ", " + sv : sv;
          if (lk === "set-cookie") (sc || (sc = [])).push(sv);
        }
      }
    }
    target[mField] = m;
    target[scField] = sc;
  }

  function Headers(init) {
    if (init === "__SLICE_LIVE_REQ__") {
      this._live = true;
      this._m = null;
      return;
    }
    this._live = false;
    if (!init) {
      this._m = Object.create(null);
      this._sc = null;
      return;
    }
    if (Array.isArray(init)) {
      this._m = Object.create(null);
      this._sc = null;
      for (var j = 0; j < init.length; j++) {
        var pair = init[j];
        if (!pair || typeof pair !== "object" || pair.length !== 2) {
          throw new TypeError("Headers: sequence item must contain exactly 2 elements");
        }
        this.append(pair[0], pair[1]);
      }
      return;
    }
    if (typeof init === "object") {
      if (!init._isHeaders && typeof Symbol !== "undefined" && Symbol.iterator && typeof init[Symbol.iterator] === "function") {
        this._m = Object.create(null);
        this._sc = null;
        var hIter = init[Symbol.iterator](), hStep;
        while (!(hStep = hIter.next()).done) {
          var hItem = hStep.value;
          if (!hItem || typeof hItem !== "object") {
            throw new TypeError("Headers: sequence item must be a 2-element pair");
          }
          var hPair = Array.isArray(hItem) ? hItem : Array.from(hItem);
          if (hPair.length !== 2) {
            throw new TypeError("Headers: sequence item must contain exactly 2 elements");
          }
          this.append(hPair[0], hPair[1]);
        }
        return;
      }
      __assignHeadersMap(this, "_m", "_sc", init);
      return;
    }
    this._m = Object.create(null);
    this._sc = null;
  }
  Headers.prototype._isHeaders = true;
  Headers.prototype._ensure = function() {
    if (this._live && !this._m) {
      var m = Object.create(null);
      this._m = m;
      var raw = JSON.parse(__slice_http_headers_all());
      for (var i = 0; i < raw.length; i++) {
        var k = raw[i][0].toLowerCase(), v = raw[i][1];
        m[k] = m[k] ? m[k] + ", " + v : v;
        if (k === "set-cookie") (this._sc || (this._sc = [])).push(v);
      }
      this._live = false;
    }
  };
  Headers.prototype.get = function(n) {
    if (typeof n === "string") {
      var lc = __hdrNameLc[n];
      if (lc !== undefined) {
        var mFast = this._m;
        if (mFast) {
          var vFast = mFast[lc];
          return vFast !== undefined ? vFast : null;
        }
        if (this._live) return __slice_http_header(lc);
        return null;
      }
    }
    var k = __lcHdrName(n);
    var m = this._m;
    if (m) {
      var v = m[k];
      return v !== undefined ? v : null;
    }
    if (this._live) return __slice_http_header(k);
    return null;
  };
  Headers.prototype.has = function(n) {
    return this.get(n) !== null;
  };
  Headers.prototype.set = function(n, v) {
    this._ensure();
    var k = __lcHdrName(n);
    var s = __normHdrVal(v);
    this._m[k] = s;
    if (k === "set-cookie") this._sc = [s];
  };
  Headers.prototype.append = function(n, v) {
    this._ensure();
    var k = __lcHdrName(n);
    var s = __normHdrVal(v);
    var cur = this._m[k];
    this._m[k] = cur !== undefined ? cur + ", " + s : s;
    if (k === "set-cookie") (this._sc || (this._sc = [])).push(s);
  };
  Headers.prototype.delete = function(n) {
    this._ensure();
    var k = __lcHdrName(n);
    delete this._m[k];
    if (k === "set-cookie") this._sc = null;
  };
  Headers.prototype.getSetCookie = function() {
    this._ensure();
    return this._sc ? this._sc.slice() : [];
  };
  var __hdrIterProto = Object.create(Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]())));
  __hdrIterProto.next = function() {
    var ps = this._t._toPairs();
    if (this._i >= ps.length) return { value: undefined, done: true };
    var pair = ps[this._i++];
    var k = this._k;
    return {
      value: k === 0 ? pair[0] : (k === 1 ? pair[1] : [pair[0], pair[1]]),
      done: false
    };
  };
  if (typeof Symbol !== "undefined" && Symbol.toStringTag) {
    Object.defineProperty(__hdrIterProto, Symbol.toStringTag, {
      value: "Headers Iterator",
      configurable: true
    });
  }
  function __mkHdrIter(hdrs, kind) {
    var it = Object.create(__hdrIterProto);
    it._t = hdrs;
    it._i = 0;
    it._k = kind;
    return it;
  }

  Headers.prototype._toPairs = function() {
    this._ensure();
    var m = this._m, sc = this._sc, ks = Object.keys(m).sort(), r = [];
    for (var i = 0; i < ks.length; i++) {
      var k = ks[i];
      if (k === "set-cookie" && sc && sc.length > 0) {
        for (var j = 0; j < sc.length; j++) r.push(["set-cookie", sc[j]]);
      } else {
        r.push([k, m[k]]);
      }
    }
    return r;
  };
  Headers.prototype.entries = function() {
    return __mkHdrIter(this, 2);
  };
  Headers.prototype.keys = function() {
    return __mkHdrIter(this, 0);
  };
  Headers.prototype.values = function() {
    return __mkHdrIter(this, 1);
  };
  Headers.prototype.forEach = function(cb, ta) {
    var ps = this._toPairs();
    for (var i = 0; i < ps.length; i++) {
      cb.call(ta, ps[i][1], ps[i][0], this);
    }
  };
  if (typeof Symbol !== "undefined" && Symbol.iterator) {
    Headers.prototype[Symbol.iterator] = Headers.prototype.entries;
  }

  function __borrowBodyBytes(obj) {
    if (obj._isLiveReq) {
      if (obj._u8 === null) {
        obj._u8 = typeof __slice_http_body_bytes === "function"
          ? __slice_http_body_bytes()
          : __slice_text_encode(__slice_http_body() || "");
      }
      return obj._u8;
    }
    if (obj._u8 instanceof Uint8Array) return obj._u8;
    return __slice_text_encode(obj._body || "");
  }

  function __extractBodyBytes(obj) {
    if (obj._isLiveReq) {
      return new Uint8Array(__borrowBodyBytes(obj));
    }
    if (obj._u8 instanceof Uint8Array) return new Uint8Array(obj._u8);
    return __slice_text_encode(obj._body || "");
  }

  function __getBodyText(obj) {
    if (obj._isLiveReq) {
      if (obj._b === null) {
        if (obj._u8 instanceof Uint8Array) {
          if (!globalThis.TextDecoder) __slice_ensure_shard(2);
          obj._b = __slice_text_decode(obj._u8, "utf-8", false, false);
        } else {
          obj._b = __slice_http_body() || "";
        }
      }
      return obj._b;
    }
    if (typeof obj._body === "string") return obj._body;
    if (obj._u8 instanceof Uint8Array) {
      if (!globalThis.TextDecoder) __slice_ensure_shard(2);
      var decoded = __slice_text_decode(obj._u8, "utf-8", false, false);
      obj._body = decoded;
      return decoded;
    }
    return "";
  }

  function __effectiveContentType(obj) {
    if (obj._h === null && obj._hm === null && !obj._isLiveReq) {
      return obj._ct || "";
    }
    return obj.headers.get("content-type") || "";
  }

  function __bodyToFormData(obj) {
    if (!globalThis.FormData) __slice_ensure_shard(15);
    return globalThis.__slice_parse_formdata(obj);
  }

  function __isReadableStreamObj(st) {
    return Boolean(st && typeof st === "object" && typeof st.getReader === "function" && typeof st._cancelInternal === "function");
  }

  function __ensureUsableStream(st) {
    if (st._disturbed || st.locked) {
      throw new TypeError("Body ReadableStream is already disturbed or locked");
    }
  }

  function __hasBody(obj) {
    if (obj._used || obj._stream !== null) return true;
    if (obj._isLiveReq) {
      var m = obj._m || (obj._m = __slice_http_method() || "GET");
      if (m !== "GET" && m !== "HEAD") return true;
      var lb = __borrowBodyBytes(obj);
      return Boolean(lb && lb.byteLength > 0);
    }
    return obj._body !== null || obj._u8 !== null;
  }

  function __isBodyUnusable(obj) {
    return Boolean(obj._used || (obj._stream !== null && (obj._stream._disturbed || obj._stream.locked)));
  }

  function __normHttpBody(rawBody, headers) {
    if (rawBody && rawBody._isUSP) {
      if (headers && !headers.has("content-type")) {
        headers.set("content-type", "application/x-www-form-urlencoded;charset=UTF-8");
      }
      return { str: rawBody.toString(), u8: null };
    }
    if (typeof rawBody === "object" && rawBody !== null) {
      if (rawBody._isFormData) {
        return globalThis.__slice_encode_formdata(rawBody, headers);
      }
      if (rawBody._isBlob) {
        if (rawBody.type && headers && !headers.has("content-type")) {
          headers.set("content-type", rawBody.type);
        }
        return { str: null, u8: new Uint8Array(rawBody._u8) };
      }
      if (
        rawBody instanceof Uint8Array ||
        rawBody instanceof ArrayBuffer ||
        (rawBody.buffer instanceof ArrayBuffer)
      ) {
        return { str: null, u8: new Uint8Array(__slice_toU8(rawBody)) };
      }
      if (typeof rawBody._toText === "function") {
        if (__streamHasBinary(rawBody) && typeof rawBody._toBytes === "function") {
          return {
            str: null,
            u8: rawBody._toBytes()
          };
        }
        return {
          str: rawBody._toText(),
          u8: null
        };
      }
    }
    if (typeof rawBody === "string") {
      if (headers && !headers.has("content-type")) {
        headers.set("content-type", "text/plain;charset=UTF-8");
      }
      return { str: rawBody, u8: null };
    }
    var coerced = String(rawBody);
    if (headers && !headers.has("content-type")) {
      headers.set("content-type", "text/plain;charset=UTF-8");
    }
    return { str: coerced, u8: null };
  }

  var __defaultReqSignal = {
    aborted: false,
    reason: undefined,
    onabort: null,
    addEventListener: function() {},
    removeEventListener: function() {},
    dispatchEvent: function() { return true; },
    throwIfAborted: function() {}
  };

  function __makeLiveReq() {
    var r = Object.create(LiveReqProto);
    r._m = null;
    r._u = null;
    r._h = null;
    r._b = null;
    r._u8 = null;
    r._stream = null;
    r.redirect = "follow";
    r.signal = __defaultReqSignal;
    r._used = false;
    return r;
  }

  function __FastHdr(m, sc) {
    this._live = false;
    this._m = m;
    this._sc = sc;
  }
  __FastHdr.prototype = Headers.prototype;

  function __FastReq(url, method, body, ct, redirect, signal) {
    this.url = url;
    this.method = method;
    this._h = null;
    this._hm = null;
    this._hsc = null;
    this._ct = ct;
    this._stream = null;
    this._body = body;
    this._u8 = null;
    this.redirect = redirect;
    this.signal = signal;
    this._used = false;
  }

  function __FastRes(status, statusText, body, ct) {
    this.status = status;
    this.statusText = statusText;
    this.ok = status >= 200 && status < 300;
    this._h = null;
    this._hm = null;
    this._hsc = null;
    this._stream = null;
    this._body = body;
    this._u8 = null;
    this._ct = ct;
    this._isSliceRes = true;
    this._used = false;
  }

  var __symIter = (typeof Symbol !== "undefined" && Symbol.iterator) || null;
