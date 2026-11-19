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

  var __HDR_CTL_RE = /[\0\r\n]/;
  var __hdrValCache = Object.create(null);
  var __hdrValCount = 0;

  function __normHdrVal(v) {
    if (typeof v === "string") {
      var cached = __hdrValCache[v];
      if (cached !== undefined) return cached;
      var len = v.length;
      if (len === 0) return "";
      if (__HDR_CTL_RE.test(v)) {
        throw new TypeError("Headers: invalid control character in header value");
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
    if (__HDR_CTL_RE.test(s)) {
      throw new TypeError("Headers: invalid control character in header value");
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
      oh._ensure();
      for (var hk in oh._m) m[hk] = oh._m[hk];
      target[mField] = m;
      target[scField] = oh._sc ? oh._sc.slice() : null;
      return;
    }
    var sc = null;
    var ks = Object.keys(oh);
    for (var k = 0; k < ks.length; k++) {
      var key = ks[k], val = oh[key];
      if (val !== undefined) {
        var lk = __lcHdrName(key), sv = __normHdrVal(val);
        m[lk] = m[lk] !== undefined ? m[lk] + ", " + sv : sv;
        if (lk === "set-cookie") (sc || (sc = [])).push(sv);
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

  function Request(input, init) {
    if (input === "__SLICE_LIVE_REQ__") {
      return __makeLiveReq();
    }
    if (!init && typeof input === "string") {
      var rSimple = Object.create(LazyReqProto);
      rSimple.url = input;
      rSimple.method = "GET";
      rSimple._h = null;
      rSimple._hm = null;
      rSimple._hsc = null;
      rSimple._ct = "";
      rSimple._stream = null;
      rSimple._body = null;
      rSimple._u8 = null;
      rSimple.redirect = "follow";
      rSimple.signal = __defaultReqSignal;
      rSimple._used = false;
      return rSimple;
    }
    var o = init || {};
    var oh = o.headers;
    if (
      typeof input === "string" &&
      (o.body === undefined || o.body === null || typeof o.body === "string") &&
      (!oh || (typeof oh === "object" && !Array.isArray(oh)))
    ) {
      var rawMFast = o.method;
      var fastMethod = rawMFast
        ? (rawMFast === "GET" || rawMFast === "POST" ? rawMFast : String(rawMFast).toUpperCase())
        : "GET";
      if (o.body !== undefined && o.body !== null && (fastMethod === "GET" || fastMethod === "HEAD")) {
        throw new TypeError("Request with GET/HEAD method cannot have body");
      }
      var rInit = Object.create(LazyReqProto);
      rInit.url = input;
      rInit.method = fastMethod;
      rInit._h = null;
      if (oh) {
        __assignHeadersMap(rInit, "_hm", "_hsc", oh);
      } else {
        rInit._hm = null;
        rInit._hsc = null;
      }
      rInit._stream = null;
      var isStrReqBody = typeof o.body === "string";
      rInit._body = isStrReqBody ? o.body : null;
      rInit._u8 = null;
      rInit._ct = (isStrReqBody && (!rInit._hm || rInit._hm["content-type"] === undefined))
        ? "text/plain;charset=UTF-8"
        : "";
      rInit.redirect = o.redirect ? String(o.redirect) : "follow";
      rInit.signal = o.signal || __defaultReqSignal;
      rInit._used = false;
      return rInit;
    }
    var isReq = Boolean(input && typeof input === "object" && input._isSliceReq);
    this.url = typeof input === "string"
      ? input
      : (isReq ? input.url : (input && input._isSliceUrl ? input.href : String(input)));
    var rawM = o.method;
    this.method = rawM
      ? (rawM === "GET" || rawM === "POST" ? rawM : String(rawM).toUpperCase())
      : (isReq ? input.method : "GET");
    this.headers = (oh && oh._isHeaders) ? new Headers(oh) : new Headers(oh || (isReq ? input.headers : undefined));
    this._stream = null;
    if (o.body !== undefined && o.body !== null) {
      if (this.method === "GET" || this.method === "HEAD") {
        throw new TypeError("Request with GET/HEAD method cannot have body");
      }
      if (typeof o.body === "string") {
        this._body = o.body;
        this._u8 = null;
        if (!this.headers._m["content-type"]) {
          this.headers._m["content-type"] = "text/plain;charset=UTF-8";
        }
      } else if (__isReadableStreamObj(o.body)) {
        __ensureUsableStream(o.body);
        this._stream = o.body;
        this._body = null;
        this._u8 = null;
      } else {
        var nb = __normHttpBody(o.body, this.headers);
        this._body = nb.str;
        this._u8 = nb.u8;
      }
    } else if (o.body === undefined && isReq && __hasBody(input)) {
      if (this.method === "GET" || this.method === "HEAD") {
        throw new TypeError("Request with GET/HEAD method cannot have body");
      }
      if (__isBodyUnusable(input)) {
        throw new TypeError("Cannot construct a Request from a Request whose body is disturbed or locked");
      }
      if (input._stream !== null) {
        if (!globalThis.ReadableStream) __slice_ensure_shard(3);
        var srcReader = input._stream.getReader();
        input._used = true;
        this._stream = new globalThis.ReadableStream({
          pull: function(c) {
            return srcReader.read().then(function(step) {
              if (step.done) c.close();
              else c.enqueue(step.value);
            });
          },
          cancel: function(reason) {
            return srcReader.cancel(reason);
          }
        }, { highWaterMark: 0 });
        this._body = null;
        this._u8 = null;
      } else {
        var inU8 = input._isLiveReq ? __borrowBodyBytes(input) : input._u8;
        if (inU8 instanceof Uint8Array) {
          this._u8 = new Uint8Array(inU8);
          this._body = null;
        } else {
          this._body = __getBodyText(input);
          this._u8 = null;
        }
        input._used = true;
      }
    } else {
      this._body = null;
      this._u8 = null;
    }
    this.redirect = o.redirect ? String(o.redirect) : (isReq && input.redirect ? input.redirect : "follow");
    this.signal = o.signal || (isReq && input.signal ? input.signal : __defaultReqSignal);
    this._used = false;
  }
  Request.prototype._isSliceReq = true;

  function Response(body, init) {
    if (!init) {
      if (body === undefined || body === null || typeof body === "string") {
        var rSimple = Object.create(LazyResProto);
        rSimple.status = 200;
        rSimple.statusText = "OK";
        rSimple.ok = true;
        rSimple._h = null;
        rSimple._hm = null;
        rSimple._hsc = null;
        rSimple._stream = null;
        var isSimpleStr = typeof body === "string";
        rSimple._body = isSimpleStr ? body : null;
        rSimple._u8 = null;
        rSimple._ct = isSimpleStr ? "text/plain;charset=UTF-8" : "";
        rSimple._isSliceRes = true;
        rSimple._used = false;
        return rSimple;
      }
      this.status = 200;
      this.statusText = "OK";
      this.ok = true;
      this.headers = new Headers();
      this._stream = null;
      if (__isReadableStreamObj(body)) {
        __ensureUsableStream(body);
        this._stream = body;
        this._body = null;
        this._u8 = null;
      } else {
        var nbFast = __normHttpBody(body, this.headers);
        this._body = nbFast.str;
        this._u8 = nbFast.u8;
      }
      this._ct = "";
      this._isSliceRes = true;
      this._used = false;
      return;
    }
    var o = init;
    var st = o.status !== undefined
      ? (typeof o.status === "number" ? o.status : Number(o.status))
      : 200;
    if ((st | 0) !== st || st < 200 || st > 599) {
      throw new RangeError(
        "Failed to construct 'Response': The status provided (" + o.status + ") is outside the range [200, 599]."
      );
    }
    var stText = o.statusText !== undefined ? String(o.statusText) : (st === 200 ? "OK" : "");
    var oh = o.headers;
    if (
      (body === undefined || body === null || typeof body === "string") &&
      (!oh || (typeof oh === "object" && !Array.isArray(oh)))
    ) {
      var rInit = Object.create(LazyResProto);
      rInit.status = st;
      rInit.statusText = stText;
      rInit.ok = st >= 200 && st < 300;
      rInit._h = null;
      if (oh) {
        __assignHeadersMap(rInit, "_hm", "_hsc", oh);
      } else {
        rInit._hm = null;
        rInit._hsc = null;
      }
      rInit._stream = null;
      var isInitStr = typeof body === "string";
      rInit._body = isInitStr ? body : null;
      rInit._u8 = null;
      rInit._ct = (isInitStr && (!rInit._hm || rInit._hm["content-type"] === undefined))
        ? "text/plain;charset=UTF-8"
        : "";
      rInit._isSliceRes = true;
      rInit._used = false;
      return rInit;
    }
    this.status = st;
    this.statusText = stText;
    this.ok = st >= 200 && st < 300;
    this.headers = (oh && oh._isHeaders) ? new Headers(oh) : new Headers(oh);
    this._stream = null;
    this._ct = "";
    if (body === undefined || body === null) {
      this._body = null;
      this._u8 = null;
    } else if (typeof body === "string") {
      this._body = body;
      this._u8 = null;
      if (!this.headers._m["content-type"]) {
        this.headers._m["content-type"] = "text/plain;charset=UTF-8";
      }
    } else if (__isReadableStreamObj(body)) {
      __ensureUsableStream(body);
      this._stream = body;
      this._body = null;
      this._u8 = null;
    } else {
      var nb = __normHttpBody(body, this.headers);
      this._body = nb.str;
      this._u8 = nb.u8;
    }
    this._isSliceRes = true;
    this._used = false;
  }

  var RP = Request.prototype, SP = Response.prototype;
  var LiveReqProto = Object.create(RP);
  LiveReqProto._isLiveReq = true;
  Object.defineProperties(LiveReqProto, {
    method: {
      get: function() { return this._m || (this._m = __slice_http_method() || "GET"); },
      set: function(v) { this._m = v; },
      configurable: true,
      enumerable: true
    },
    url: {
      get: function() { return this._u || (this._u = __slice_http_url()); },
      set: function(v) { this._u = v; },
      configurable: true,
      enumerable: true
    },
    headers: {
      get: function() { return this._h || (this._h = new Headers("__SLICE_LIVE_REQ__")); },
      set: function(v) { this._h = v; },
      configurable: true,
      enumerable: true
    },
    _body: {
      get: function() { return __getBodyText(this); },
      set: function(v) { this._b = v; },
      configurable: true,
      enumerable: false
    }
  });
  var LazyReqProto = Object.create(RP);
  Object.defineProperty(LazyReqProto, "headers", {
    get: function() {
      if (!this._h) {
        var h = Object.create(Headers.prototype);
        h._live = false;
        var hm = this._hm || Object.create(null);
        if (this._ct && hm["content-type"] === undefined) hm["content-type"] = this._ct;
        h._m = hm;
        h._sc = this._hsc || null;
        this._h = h;
        this._hm = null;
        this._hsc = null;
        this._ct = "";
      }
      return this._h;
    },
    set: function(v) {
      this._h = v;
      this._hm = null;
      this._hsc = null;
      this._ct = "";
    },
    configurable: true,
    enumerable: true
  });
  var LazyResProto = Object.create(SP);
  Object.defineProperty(LazyResProto, "headers", {
    get: function() {
      if (!this._h) {
        var h = Object.create(Headers.prototype);
        h._live = false;
        var hm = this._hm || Object.create(null);
        if (this._ct && hm["content-type"] === undefined) hm["content-type"] = this._ct;
        h._m = hm;
        h._sc = this._hsc || null;
        this._h = h;
        this._hm = null;
        this._hsc = null;
        this._ct = "";
      }
      return this._h;
    },
    set: function(v) {
      this._h = v;
      this._hm = null;
      this._hsc = null;
      this._ct = "";
    },
    configurable: true,
    enumerable: true
  });

  RP._borrowBytes = SP._borrowBytes = function() {
    return __borrowBodyBytes(this);
  };
  RP._getText = SP._getText = function() {
    return __getBodyText(this);
  };

  var __bodyUsedDesc = {
    get: function() {
      return Boolean(this._used || (this._stream !== null && this._stream._disturbed));
    },
    configurable: true,
    enumerable: true
  };
  Object.defineProperty(RP, "bodyUsed", __bodyUsedDesc);
  Object.defineProperty(SP, "bodyUsed", __bodyUsedDesc);

  RP.text = SP.text = function() {
    if (__isBodyUnusable(this)) {
      return Promise.reject(new TypeError("Body has already been consumed or is locked"));
    }
    if (!__hasBody(this)) {
      return Promise.resolve("");
    }
    this._used = true;
    if (this._stream) {
      return this._stream._drainToBody(this).then(__getBodyText);
    }
    return Promise.resolve(__getBodyText(this));
  };
  RP.json = SP.json = function() {
    if (__isBodyUnusable(this)) {
      return Promise.reject(new TypeError("Body has already been consumed or is locked"));
    }
    if (!__hasBody(this)) {
      try {
        return Promise.resolve(JSON.parse(""));
      } catch (eEmpty) {
        return Promise.reject(eEmpty);
      }
    }
    this._used = true;
    if (this._stream) {
      return this._stream._drainToBody(this).then(function(s) {
        return JSON.parse(__getBodyText(s));
      });
    }
    try {
      return Promise.resolve(JSON.parse(__getBodyText(this)));
    } catch (e) {
      return Promise.reject(e);
    }
  };
  RP.textSync = SP.textSync = function() {
    if (__isBodyUnusable(this)) {
      throw new TypeError("Body has already been consumed or is locked");
    }
    if (!__hasBody(this)) {
      return "";
    }
    this._used = true;
    if (this._stream) {
      var st = this._stream;
      st.getReader();
      st._disturbed = true;
      if (__streamHasBinary(st) && typeof st._toBytes === "function") {
        this._u8 = st._toBytes();
        this._body = null;
      } else {
        this._body = st._toText();
      }
    }
    return __getBodyText(this);
  };
  RP.jsonSync = SP.jsonSync = function() {
    return JSON.parse(this.textSync());
  };
  RP.arrayBuffer = SP.arrayBuffer = function() {
    if (__isBodyUnusable(this)) {
      return Promise.reject(new TypeError("Body has already been consumed or is locked"));
    }
    if (!__hasBody(this)) {
      return Promise.resolve(new ArrayBuffer(0));
    }
    this._used = true;
    if (this._stream) {
      return this._stream._drainToBody(this).then(function(s) { return __extractBodyBytes(s).buffer; });
    }
    return Promise.resolve(__extractBodyBytes(this).buffer);
  };
  RP.bytes = SP.bytes = function() {
    if (__isBodyUnusable(this)) {
      return Promise.reject(new TypeError("Body has already been consumed or is locked"));
    }
    if (!__hasBody(this)) {
      return Promise.resolve(new Uint8Array(0));
    }
    this._used = true;
    if (this._stream) {
      return this._stream._drainToBody(this).then(function(s) { return __extractBodyBytes(s); });
    }
    return Promise.resolve(__extractBodyBytes(this));
  };
  RP.blob = SP.blob = function() {
    if (__isBodyUnusable(this)) {
      return Promise.reject(new TypeError("Body has already been consumed or is locked"));
    }
    if (!globalThis.Blob) __slice_ensure_shard(15);
    var cType = __effectiveContentType(this);
    if (!__hasBody(this)) {
      return Promise.resolve(new globalThis.Blob([], { type: cType }));
    }
    this._used = true;
    if (this._stream) {
      return this._stream._drainToBody(this).then(function(s) {
        return new globalThis.Blob([__extractBodyBytes(s)], { type: cType });
      });
    }
    return Promise.resolve(new globalThis.Blob([__extractBodyBytes(this)], { type: cType }));
  };
  RP.formData = SP.formData = function() {
    if (__isBodyUnusable(this)) {
      return Promise.reject(new TypeError("Body has already been consumed or is locked"));
    }
    if (!__hasBody(this)) {
      try {
        return Promise.resolve(__bodyToFormData(this));
      } catch (eEmpty) {
        return Promise.reject(eEmpty);
      }
    }
    this._used = true;
    if (this._stream) {
      return this._stream._drainToBody(this).then(function(s) { return __bodyToFormData(s); });
    }
    try {
      return Promise.resolve(__bodyToFormData(this));
    } catch (e) {
      return Promise.reject(e);
    }
  };
  RP.clone = function() {
    if (__isBodyUnusable(this)) {
      throw new TypeError("Cannot clone a disturbed or locked Request");
    }
    if (!__hasBody(this)) {
      return new Request(this.url, {
        method: this.method,
        headers: new Headers(this.headers),
        body: null,
        redirect: this.redirect,
        signal: this.signal
      });
    }
    var stClone = null;
    if (this._stream && typeof this._stream.tee === "function") {
      var pair = this._stream.tee();
      this._stream = pair[0];
      stClone = pair[1];
    }
    var liveU8 = this._isLiveReq ? __borrowBodyBytes(this) : this._u8;
    var hasBin = !stClone && (liveU8 instanceof Uint8Array);
    var r = new Request(this.url, {
      method: this.method,
      headers: new Headers(this.headers),
      body: stClone ? stClone : (hasBin ? null : __getBodyText(this)),
      redirect: this.redirect,
      signal: this.signal
    });
    if (hasBin) {
      r._u8 = new Uint8Array(liveU8);
      r._body = null;
    }
    return r;
  };
  SP.clone = function() {
    if (__isBodyUnusable(this)) {
      throw new TypeError("Cannot clone a disturbed or locked Response");
    }
    if (this.status < 200 || this.status > 599) {
      var sr = __makeSpecialResponse(this.status, this.statusText, null);
      sr._h = new Headers(this.headers);
      if (this.type) sr.type = this.type;
      return sr;
    }
    if (!__hasBody(this)) {
      return new Response(null, {
        status: this.status,
        statusText: this.statusText,
        headers: new Headers(this.headers)
      });
    }
    var stClone = null;
    if (this._stream && typeof this._stream.tee === "function") {
      var pair = this._stream.tee();
      this._stream = pair[0];
      stClone = pair[1];
    }
    var hasBin = !stClone && (this._u8 instanceof Uint8Array);
    var r = new Response(stClone ? stClone : (hasBin ? null : this._body), {
      status: this.status,
      statusText: this.statusText,
      headers: new Headers(this.headers)
    });
    if (hasBin) {
      r._u8 = new Uint8Array(this._u8);
      r._body = null;
    }
    return r;
  };
  var __bodyDesc = {
    get: function() {
      if (this._stream) return this._stream;
      if (!__hasBody(this)) return null;
      if (!globalThis.ReadableStream) __slice_ensure_shard(3);
      if (this._used) {
        var emptyRs = new globalThis.ReadableStream({
          start: function(c) { c.close(); }
        });
        emptyRs._disturbed = true;
        emptyRs.getReader();
        this._stream = emptyRs;
        return emptyRs;
      }
      var b = __extractBodyBytes(this);
      var rs = new globalThis.ReadableStream({
        start: function(c) {
          if (b.byteLength > 0) c.enqueue(b);
          c.close();
        }
      });
      this._stream = rs;
      return rs;
    },
    configurable: true,
    enumerable: true
  };
  Object.defineProperty(RP, "body", __bodyDesc);
  Object.defineProperty(SP, "body", __bodyDesc);

  function __makeSpecialResponse(status, statusText, rawHeaders) {
    var r = Object.create(LazyResProto);
    r.status = status;
    r.statusText = statusText || "";
    r.ok = status >= 200 && status < 300;
    r._h = null;
    if (rawHeaders) {
      __assignHeadersMap(r, "_hm", "_hsc", rawHeaders);
    } else {
      r._hm = null;
      r._hsc = null;
    }
    r._stream = null;
    r._body = null;
    r._u8 = null;
    r._ct = "";
    r._isSliceRes = true;
    r._used = false;
    return r;
  }
  globalThis.__slice_make_special_response = __makeSpecialResponse;

  Response.json = function(data, init) {
    var jsonStr = JSON.stringify(data);
    if (jsonStr === undefined) {
      throw new TypeError("Value is not JSON serializable");
    }
    if (!init) {
      var rFast = Object.create(LazyResProto);
      rFast.status = 200;
      rFast.statusText = "OK";
      rFast.ok = true;
      rFast._h = null;
      rFast._hm = null;
      rFast._hsc = null;
      rFast._stream = null;
      rFast._body = jsonStr;
      rFast._u8 = null;
      rFast._ct = "application/json";
      rFast._isSliceRes = true;
      rFast._used = false;
      return rFast;
    }
    if (typeof init === "object") {
      var jSt = init.status !== undefined
        ? (typeof init.status === "number" ? init.status : Number(init.status))
        : 200;
      var jOh = init.headers;
      if (
        (jSt | 0) === jSt &&
        jSt >= 200 &&
        jSt <= 599 &&
        (!jOh || (typeof jOh === "object" && !Array.isArray(jOh)))
      ) {
        var rJson = Object.create(LazyResProto);
        rJson.status = jSt;
        rJson.statusText = init.statusText !== undefined ? String(init.statusText) : (jSt === 200 ? "OK" : "");
        rJson.ok = jSt >= 200 && jSt < 300;
        rJson._h = null;
        if (jOh) {
          __assignHeadersMap(rJson, "_hm", "_hsc", jOh);
          rJson._ct = rJson._hm["content-type"] === undefined ? "application/json" : "";
        } else {
          rJson._hm = null;
          rJson._hsc = null;
          rJson._ct = "application/json";
        }
        rJson._stream = null;
        rJson._body = jsonStr;
        rJson._u8 = null;
        rJson._isSliceRes = true;
        rJson._used = false;
        return rJson;
      }
    }
    var r = new Response(null, typeof init === "number" ? { status: init } : init);
    r._body = jsonStr;
    if (r._h === null) {
      if (!r._hm || r._hm["content-type"] === undefined) r._ct = "application/json";
    } else if (!r.headers._m["content-type"]) {
      r.headers._m["content-type"] = "application/json";
    }
    return r;
  };
  Response.redirect = function(url, status) {
    var st = status === undefined ? 302 : Number(status);
    if (st !== 301 && st !== 302 && st !== 303 && st !== 307 && st !== 308) {
      throw new RangeError("Failed to execute 'redirect' on 'Response': Invalid redirect status code.");
    }
    return new Response(null, { status: st, headers: { location: String(url) } });
  };
  Response.error = function() {
    var r = __makeSpecialResponse(0, "", null);
    r.type = "error";
    return r;
  };

  function __resHdrJson(h) {
    if (!h || !h._m) return "";
    var ks = Object.keys(h._m), out = [];
    for (var i = 0; i < ks.length; i++) {
      var k = ks[i];
      if (k === "content-type" || k === "content-length") continue;
      if (k === "set-cookie" && h._sc && h._sc.length > 1) {
        for (var j = 0; j < h._sc.length; j++) out.push(["set-cookie", h._sc[j]]);
      } else {
        out.push([k, h._m[k]]);
      }
    }
    return out.length > 0 ? JSON.stringify(out) : "";
  }

  function __streamHasBinary(st) {
    if (typeof st._drainSync === "function") st._drainSync();
    var ch = st._chunks;
    if (!ch) return false;
    for (var i = st._head || 0; i < ch.length; i++) {
      if (typeof ch[i] !== "string") return true;
    }
    return false;
  }

  function __finishSliceRes(r) {
    var isNoBodyStatus = r.status === 101 || r.status === 204 || r.status === 304;
    if (r._h === null && !r._stream) {
      var ctFast = r._ct || (isNoBodyStatus ? "" : (r._u8 instanceof Uint8Array ? "application/octet-stream" : "text/plain;charset=UTF-8"));
      var hdrJsonFast = "";
      var hm = r._hm;
      if (hm) {
        if (hm["content-type"] !== undefined) ctFast = hm["content-type"];
        var extra = null;
        for (var lk in hm) {
          if (lk === "content-type" || lk === "content-length") continue;
          if (!extra) extra = [];
          if (lk === "set-cookie" && r._hsc && r._hsc.length > 1) {
            for (var scIdx = 0; scIdx < r._hsc.length; scIdx++) extra.push(["set-cookie", r._hsc[scIdx]]);
          } else {
            extra.push([lk, hm[lk]]);
          }
        }
        if (extra) hdrJsonFast = JSON.stringify(extra);
      }
      if (r._u8 instanceof Uint8Array) {
        __slice_http_respond(r.status, r._u8, ctFast, hdrJsonFast);
        return;
      }
      __slice_http_respond(r.status, r._body || "", ctFast, hdrJsonFast);
      return;
    }
    var h = r._h !== undefined && r._h !== null ? r._h : r.headers;
    var hasBin = r._u8 instanceof Uint8Array;
    var ct = (h && h._m && h._m["content-type"]) || r._ct || (isNoBodyStatus ? "" : (hasBin ? "application/octet-stream" : "text/plain;charset=UTF-8"));
    var hdrJson = h ? __resHdrJson(h) : "";
    if (r._stream) {
      var st = r._stream;
      if (typeof st._isAsyncPending === "function" && st._isAsyncPending()) {
        st._drainToBody(r).then(function() {
          var hs = r._h !== undefined && r._h !== null ? r._h : r.headers;
          var isBinS = r._u8 instanceof Uint8Array;
          var ctS = (hs && hs._m && hs._m["content-type"]) || r._ct || (isBinS ? "application/octet-stream" : "text/plain;charset=UTF-8");
          __slice_http_respond(
            r.status,
            isBinS ? r._u8 : (r._body || ""),
            ctS,
            hs ? __resHdrJson(hs) : ""
          );
        });
        return;
      }
      var isBin = __streamHasBinary(st);
      var ctStream = (h && h._m && h._m["content-type"]) || r._ct || (isBin ? "application/octet-stream" : "text/plain;charset=UTF-8");
      __slice_http_respond(
        r.status,
        isBin && typeof st._toBytes === "function" ? st._toBytes() : st._toText(),
        ctStream,
        hdrJson
      );
      return;
    }
    if (hasBin) {
      __slice_http_respond(r.status, r._u8, ct, hdrJson);
      return;
    }
    __slice_http_respond(r.status, r._body || "", ct, hdrJson);
  }

  globalThis.Headers = Headers;
  globalThis.Request = Request;
  globalThis.Response = Response;

  var __slice_web_ctx = {
    waitUntil: globalThis.Slice ? globalThis.Slice.waitUntil : function() {},
    passThroughOnException: function() {}
  };
  globalThis.__slice_make_web_request = __makeLiveReq;
  globalThis.__slice_dispatch_fetch = function(h) {
    if (!__slice_http_is_active()) return;
    var fn = typeof h.fetch === "function" ? h.fetch : (typeof h === "function" ? h : null);
    if (!fn) return;
    var res = fn.length <= 1
      ? fn.call(h, __makeLiveReq())
      : fn.call(h, __makeLiveReq(), globalThis.process.env, __slice_web_ctx);
    if (res) {
      if (res._isSliceRes) {
        __finishSliceRes(res);
      } else if (typeof res.then === "function") {
        res.then(function(r) {
          if (r && r._isSliceRes) __finishSliceRes(r);
        });
      }
    }
  };
})();
