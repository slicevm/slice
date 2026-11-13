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
        if (init[j] && init[j].length >= 2) this.append(init[j][0], init[j][1]);
      }
      return;
    }
    if (typeof init === "object") {
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
  Headers.prototype.entries = function() {
    this._ensure();
    var ks = Object.keys(this._m).sort(), r = [];
    for (var i = 0; i < ks.length; i++) r.push([ks[i], this._m[ks[i]]]);
    return r;
  };
  Headers.prototype.keys = function() {
    this._ensure();
    return Object.keys(this._m).sort();
  };
  Headers.prototype.values = function() {
    var es = this.entries(), r = [];
    for (var i = 0; i < es.length; i++) r.push(es[i][1]);
    return r;
  };
  Headers.prototype.forEach = function(cb, ta) {
    this._ensure();
    var m = this._m, ks = Object.keys(m);
    for (var i = 0; i < ks.length; i++) {
      var k = ks[i];
      cb.call(ta, m[k], k, this);
    }
  };
  if (typeof Symbol !== "undefined" && Symbol.iterator) {
    Headers.prototype[Symbol.iterator] = function() {
      return this.entries()[Symbol.iterator]();
    };
  }

  function __extractBodyBytes(obj) {
    if (obj._u8 instanceof Uint8Array) return new Uint8Array(obj._u8);
    var s = obj._body || (obj._data !== undefined ? JSON.stringify(obj._data) : "");
    return __slice_text_encode(s);
  }

  function __bodyToFormData(obj) {
    if (obj._fd && obj._fd._isFormData) return obj._fd;
    if (!globalThis.FormData) __slice_ensure_shard(15);
    return globalThis.__slice_parse_formdata(obj);
  }

  function __normHttpBody(rawBody, headers) {
    if (rawBody && rawBody._isUSP) {
      if (headers && !headers.has("content-type")) {
        headers.set("content-type", "application/x-www-form-urlencoded;charset=UTF-8");
      }
      return { str: rawBody.toString(), u8: null, fd: null };
    }
    if (typeof rawBody === "object" && rawBody !== null) {
      if (rawBody._isFormData) {
        return globalThis.__slice_encode_formdata(rawBody, headers);
      }
      if (rawBody._isBlob) {
        if (rawBody.type && headers && !headers.has("content-type")) {
          headers.set("content-type", rawBody.type);
        }
        if (!globalThis.TextDecoder) __slice_ensure_shard(2);
        var u8b = new Uint8Array(rawBody._u8);
        return { str: __slice_text_decode(u8b, "utf-8", false, false), u8: u8b, fd: null };
      }
      if (
        rawBody instanceof Uint8Array ||
        rawBody instanceof ArrayBuffer ||
        (rawBody.buffer instanceof ArrayBuffer)
      ) {
        if (!globalThis.TextDecoder) __slice_ensure_shard(2);
        var u8 = new Uint8Array(__slice_toU8(rawBody));
        return { str: __slice_text_decode(u8, "utf-8", false, false), u8: u8, fd: null };
      }
      if (typeof rawBody._toText === "function") {
        if (__streamHasBinary(rawBody) && typeof rawBody._toBytes === "function") {
          var stU8 = rawBody._toBytes();
          if (!globalThis.TextDecoder) __slice_ensure_shard(2);
          return {
            str: __slice_text_decode(stU8, "utf-8", false, false),
            u8: stU8,
            fd: null
          };
        }
        var txt = rawBody._toText();
        return {
          str: txt,
          u8: null,
          fd: null
        };
      }
    }
    return { str: JSON.stringify(rawBody), u8: null, fd: null };
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
    r._fd = null;
    r.redirect = "follow";
    r.signal = __defaultReqSignal;
    r.bodyUsed = false;
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
      rSimple._body = "";
      rSimple._u8 = null;
      rSimple._fd = null;
      rSimple.redirect = "follow";
      rSimple.signal = __defaultReqSignal;
      rSimple.bodyUsed = false;
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
      var rInit = Object.create(LazyReqProto);
      rInit.url = input;
      rInit.method = rawMFast
        ? (rawMFast === "GET" || rawMFast === "POST" ? rawMFast : String(rawMFast).toUpperCase())
        : "GET";
      rInit._h = null;
      if (oh) {
        __assignHeadersMap(rInit, "_hm", "_hsc", oh);
      } else {
        rInit._hm = null;
        rInit._hsc = null;
      }
      rInit._stream = null;
      rInit._body = typeof o.body === "string" ? o.body : "";
      rInit._u8 = null;
      rInit._fd = null;
      rInit.redirect = o.redirect ? String(o.redirect) : "follow";
      rInit.signal = o.signal || __defaultReqSignal;
      rInit.bodyUsed = false;
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
      if (typeof o.body === "string") {
        this._body = o.body;
        this._u8 = null;
        this._fd = null;
      } else if (typeof o.body === "object" && typeof o.body._toText === "function" && (!o.body._closed || o.body._startPromise)) {
        this._stream = o.body;
        this._body = "";
        this._u8 = null;
        this._fd = null;
      } else {
        var nb = __normHttpBody(o.body, this.headers);
        this._body = nb.str;
        this._u8 = nb.u8;
        this._fd = nb.fd;
      }
    } else if (isReq) {
      this._stream = input._stream || null;
      this._body = input._body;
      this._u8 = input._u8 ? new Uint8Array(input._u8) : null;
      this._fd = input._fd || null;
    } else {
      this._body = "";
      this._u8 = null;
      this._fd = null;
    }
    this.redirect = o.redirect ? String(o.redirect) : (isReq && input.redirect ? input.redirect : "follow");
    this.signal = o.signal || (isReq && input.signal ? input.signal : __defaultReqSignal);
    this.bodyUsed = false;
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
        rSimple._data = undefined;
        rSimple._stream = null;
        rSimple._body = typeof body === "string" ? body : "";
        rSimple._u8 = null;
        rSimple._fd = null;
        rSimple._ct = "text/plain;charset=UTF-8";
        rSimple._isSliceRes = true;
        rSimple.bodyUsed = false;
        return rSimple;
      }
      this.status = 200;
      this.statusText = "OK";
      this.ok = true;
      this.headers = new Headers();
      this._data = undefined;
      this._stream = null;
      if (typeof body === "object" && typeof body._toText === "function" && (!body._closed || body._startPromise)) {
        this._stream = body;
        this._body = "";
        this._u8 = null;
        this._fd = null;
        this.headers._m["content-type"] = "text/plain;charset=UTF-8";
      } else {
        var nbFast = __normHttpBody(body, this.headers);
        this._body = nbFast.str;
        this._u8 = nbFast.u8;
        this._fd = nbFast.fd;
      }
      this._isSliceRes = true;
      this.bodyUsed = false;
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
      rInit._data = undefined;
      rInit._stream = null;
      rInit._body = typeof body === "string" ? body : "";
      rInit._u8 = null;
      rInit._fd = null;
      rInit._ct = typeof body === "string" ? "text/plain;charset=UTF-8" : "";
      rInit._isSliceRes = true;
      rInit.bodyUsed = false;
      return rInit;
    }
    this.status = st;
    this.statusText = stText;
    this.ok = st >= 200 && st < 300;
    this.headers = (oh && oh._isHeaders) ? new Headers(oh) : new Headers(oh);
    this._data = undefined;
    this._stream = null;
    if (body === undefined || body === null) {
      this._body = "";
      this._u8 = null;
      this._fd = null;
    } else if (typeof body === "string") {
      this._body = body;
      this._u8 = null;
      this._fd = null;
      if (!this.headers._m["content-type"]) {
        this.headers._m["content-type"] = "text/plain;charset=UTF-8";
      }
    } else if (typeof body === "object" && typeof body._toText === "function" && (!body._closed || body._startPromise)) {
      this._stream = body;
      this._body = "";
      this._u8 = null;
      this._fd = null;
    } else {
      var nb = __normHttpBody(body, this.headers);
      this._body = nb.str;
      this._u8 = nb.u8;
      this._fd = nb.fd;
    }
    this._isSliceRes = true;
    this.bodyUsed = false;
  }

  var RP = Request.prototype, SP = Response.prototype;
  var LiveReqProto = Object.create(RP);
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
      get: function() {
        if (this._b === null) this._b = __slice_http_body() || "";
        return this._b;
      },
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
        h._m = this._hm || Object.create(null);
        h._sc = this._hsc || null;
        this._h = h;
        this._hm = null;
        this._hsc = null;
      }
      return this._h;
    },
    set: function(v) {
      this._h = v;
      this._hm = null;
      this._hsc = null;
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
      }
      return this._h;
    },
    set: function(v) {
      this._h = v;
      this._hm = null;
      this._hsc = null;
    },
    configurable: true,
    enumerable: true
  });

  RP.text = SP.text = function() {
    this.bodyUsed = true;
    if (this._stream) {
      return this._stream._drainToBody(this).then(function(s) { return s._body; });
    }
    if (this._data !== undefined && !this._body) this._body = JSON.stringify(this._data);
    return Promise.resolve(this._body);
  };
  RP.json = SP.json = function() {
    this.bodyUsed = true;
    if (this._stream) {
      return this._stream._drainToBody(this).then(function(s) { return JSON.parse(s._body); });
    }
    if (this._data !== undefined) return Promise.resolve(this._data);
    try { return Promise.resolve(JSON.parse(this._body)); } catch (e) { return Promise.reject(e); }
  };
  RP.textSync = SP.textSync = function() {
    this.bodyUsed = true;
    if (this._stream) {
      this._body = this._stream._toText();
      this._stream = null;
    }
    if (this._data !== undefined && !this._body) this._body = JSON.stringify(this._data);
    return this._body;
  };
  RP.jsonSync = SP.jsonSync = function() {
    this.bodyUsed = true;
    if (this._stream) {
      this._body = this._stream._toText();
      this._stream = null;
    }
    if (this._data !== undefined) return this._data;
    return JSON.parse(this._body);
  };
  RP.arrayBuffer = SP.arrayBuffer = function() {
    this.bodyUsed = true;
    if (this._stream) {
      return this._stream._drainToBody(this).then(function(s) { return __extractBodyBytes(s).buffer; });
    }
    return Promise.resolve(__extractBodyBytes(this).buffer);
  };
  RP.bytes = SP.bytes = function() {
    this.bodyUsed = true;
    if (this._stream) {
      return this._stream._drainToBody(this).then(function(s) { return __extractBodyBytes(s); });
    }
    return Promise.resolve(__extractBodyBytes(this));
  };
  RP.blob = SP.blob = function() {
    this.bodyUsed = true;
    if (!globalThis.Blob) __slice_ensure_shard(15);
    var cType = this._ct || this.headers.get("content-type") || "";
    if (this._stream) {
      return this._stream._drainToBody(this).then(function(s) {
        return new globalThis.Blob([__extractBodyBytes(s)], { type: cType });
      });
    }
    return Promise.resolve(new globalThis.Blob([__extractBodyBytes(this)], { type: cType }));
  };
  RP.formData = SP.formData = function() {
    this.bodyUsed = true;
    if (this._stream) {
      return this._stream._drainToBody(this).then(function(s) { return __bodyToFormData(s); });
    }
    return Promise.resolve(__bodyToFormData(this));
  };
  RP.clone = function() {
    var stClone = null;
    if (this._stream && typeof this._stream.tee === "function") {
      var pair = this._stream.tee();
      this._stream = pair[0];
      stClone = pair[1];
    }
    var r = new Request(this.url, {
      method: this.method,
      headers: new Headers(this.headers),
      body: stClone || this._body,
      redirect: this.redirect,
      signal: this.signal
    });
    if (this._u8) r._u8 = new Uint8Array(this._u8);
    if (this._fd) r._fd = this._fd;
    return r;
  };
  SP.clone = function() {
    if (this.status < 200 || this.status > 599) {
      var sr = __makeSpecialResponse(this.status, this.statusText, null);
      sr._h = new Headers(this.headers);
      if (this.type) sr.type = this.type;
      return sr;
    }
    var stClone = null;
    if (this._stream && typeof this._stream.tee === "function") {
      var pair = this._stream.tee();
      this._stream = pair[0];
      stClone = pair[1];
    }
    if (this._data !== undefined && !this._body) this._body = JSON.stringify(this._data);
    var r = new Response(stClone || this._body, {
      status: this.status,
      statusText: this.statusText,
      headers: new Headers(this.headers)
    });
    if (this._u8) r._u8 = new Uint8Array(this._u8);
    if (this._fd) r._fd = this._fd;
    return r;
  };
  var __bodyDesc = {
    get: function() {
      if (this._stream) return this._stream;
      if (!this._body && !this._u8 && this._data === undefined) return null;
      if (!globalThis.ReadableStream) __slice_ensure_shard(3);
      var b = __extractBodyBytes(this);
      return new globalThis.ReadableStream({
        start: function(c) {
          if (b.byteLength > 0) c.enqueue(b);
          c.close();
        }
      });
    }
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
    r._data = undefined;
    r._stream = null;
    r._body = "";
    r._u8 = null;
    r._fd = null;
    r._ct = "";
    r._isSliceRes = true;
    r.bodyUsed = false;
    return r;
  }
  globalThis.__slice_make_special_response = __makeSpecialResponse;
  Response.json = function(data, init) {
    if (!init) {
      var rFast = Object.create(LazyResProto);
      rFast.status = 200;
      rFast.statusText = "OK";
      rFast.ok = true;
      rFast._h = null;
      rFast._hm = null;
      rFast._hsc = null;
      rFast._data = data;
      rFast._stream = null;
      rFast._body = "";
      rFast._u8 = null;
      rFast._fd = null;
      rFast._ct = "application/json";
      rFast._isSliceRes = true;
      rFast.bodyUsed = false;
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
        rJson._data = data;
        rJson._stream = null;
        rJson._body = "";
        rJson._u8 = null;
        rJson._fd = null;
        rJson._isSliceRes = true;
        rJson.bodyUsed = false;
        return rJson;
      }
    }
    var r = new Response(null, typeof init === "number" ? { status: init } : init);
    if (r._h === null) {
      if (!r._hm || r._hm["content-type"] === undefined) r._ct = "application/json";
    } else if (!r.headers._m["content-type"]) {
      r.headers._m["content-type"] = "application/json";
    }
    r._data = data;
    return r;
  };
  Response.redirect = function(url, status) {
    var st = status || 302;
    return new Response("", { status: st, headers: { location: String(url) } });
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
      var rbFast = r._body || (r._data !== undefined ? JSON.stringify(r._data) : "");
      __slice_http_respond(r.status, rbFast, ctFast, hdrJsonFast);
      return;
    }
    var h = r._h !== undefined ? r._h : r.headers;
    var hasBin = r._u8 instanceof Uint8Array;
    var ct = (h && h._m && h._m["content-type"]) || r._ct || (isNoBodyStatus ? "" : (hasBin ? "application/octet-stream" : "text/plain;charset=UTF-8"));
    var hdrJson = h ? __resHdrJson(h) : "";
    if (r._stream) {
      var st = r._stream;
      if (st._startPromise) {
        st._startPromise.then(function() {
          var hs = r._h !== undefined ? r._h : r.headers;
          var isBinS = __streamHasBinary(st);
          var ctS = (hs && hs._m && hs._m["content-type"]) || r._ct || (isBinS ? "application/octet-stream" : "text/plain;charset=UTF-8");
          __slice_http_respond(
            r.status,
            isBinS && typeof st._toBytes === "function" ? st._toBytes() : st._toText(),
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
    var rb = r._body || (r._data !== undefined ? JSON.stringify(r._data) : "");
    __slice_http_respond(r.status, rb, ct, hdrJson);
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
