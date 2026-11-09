// Slice (Nexus-JS) Lazy Prelude Shard 11: WinterTC `Headers`, `Request`, `Response` (`11_fetch_headers_req_res.js`)
(function() {
  function Headers(init) {
    if (init === "__SLICE_LIVE_REQ__") {
      this._live = true;
      this._m = null;
      return;
    }
    this._live = false;
    this._sc = null;
    var m = Object.create(null);
    this._m = m;
    if (!init) return;
    if (init._isHeaders) {
      if (init._m) {
        var ik = Object.keys(init._m);
        for (var n = 0; n < ik.length; n++) m[ik[n]] = init._m[ik[n]];
        if (init._sc) this._sc = init._sc.slice();
      } else {
        var es = init.entries();
        for (var i = 0; i < es.length; i++) m[es[i][0]] = es[i][1];
      }
    } else if (Array.isArray(init)) {
      for (var j = 0; j < init.length; j++) {
        if (init[j] && init[j].length >= 2) this.append(init[j][0], init[j][1]);
      }
    } else if (typeof init === "object") {
      var ks = Object.keys(init);
      for (var k = 0; k < ks.length; k++) {
        var key = ks[k], val = init[key];
        if (val !== undefined) {
          var lk = key.toLowerCase(), sv = typeof val === "string" ? val : String(val);
          m[lk] = sv;
          if (lk === "set-cookie") this._sc = [sv];
        }
      }
    }
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
    var s = typeof n === "string" ? n : String(n);
    var m = this._m;
    if (m) {
      var v = m[s];
      if (v !== undefined) return v;
      var k = s.toLowerCase();
      if (k !== s) {
        var v2 = m[k];
        if (v2 !== undefined) return v2;
      }
      return null;
    }
    if (this._live) return __slice_http_header(s.toLowerCase());
    return null;
  };
  Headers.prototype.has = function(n) {
    return this.get(n) !== null;
  };
  Headers.prototype.set = function(n, v) {
    this._ensure();
    var k = (typeof n === "string" ? n : String(n)).toLowerCase();
    var s = typeof v === "string" ? v : String(v);
    this._m[k] = s;
    if (k === "set-cookie") this._sc = [s];
  };
  Headers.prototype.append = function(n, v) {
    this._ensure();
    var k = (typeof n === "string" ? n : String(n)).toLowerCase();
    var s = typeof v === "string" ? v : String(v);
    var cur = this._m[k];
    this._m[k] = cur !== undefined ? cur + ", " + s : s;
    if (k === "set-cookie") (this._sc || (this._sc = [])).push(s);
  };
  Headers.prototype.delete = function(n) {
    this._ensure();
    var k = (typeof n === "string" ? n : String(n)).toLowerCase();
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
    if (!globalThis.URLSearchParams) __slice_ensure_shard(1);
    var fd = new globalThis.FormData();
    var usp = new globalThis.URLSearchParams(obj._body || "");
    var es = usp.entries();
    for (var i = 0; i < es.length; i++) fd.append(es[i][0], es[i][1]);
    return fd;
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
        if (headers && !headers.has("content-type")) {
          headers.set("content-type", "multipart/form-data; boundary=----SliceFormBoundary");
        }
        var pairs = [], es = rawBody.entries();
        for (var i = 0; i < es.length; i++) {
          var v = es[i][1];
          if (v && v._isBlob && !globalThis.TextDecoder) __slice_ensure_shard(2);
          pairs.push(
            encodeURIComponent(es[i][0]) +
            "=" +
            encodeURIComponent(v && v._isBlob ? __slice_text_decode(v._u8, "utf-8", false, false) : String(v))
          );
        }
        return { str: pairs.join("&"), u8: null, fd: rawBody };
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
        var txt = rawBody._toText();
        return {
          str: txt,
          u8: typeof rawBody._toBytes === "function" ? rawBody._toBytes() : __slice_text_encode(txt),
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
      this.url = input;
      this.method = "GET";
      this.headers = new Headers();
      this._body = "";
      this._u8 = null;
      this._fd = null;
      this.redirect = "follow";
      this.signal = __defaultReqSignal;
      this.bodyUsed = false;
      return;
    }
    var o = init || {};
    var isReq = Boolean(input && typeof input === "object" && input._isSliceReq);
    this.url = typeof input === "string"
      ? input
      : (isReq ? input.url : (input && input._isSliceUrl ? input.href : String(input)));
    var rawM = o.method;
    this.method = rawM
      ? (rawM === "GET" || rawM === "POST" ? rawM : String(rawM).toUpperCase())
      : (isReq ? input.method : "GET");
    var oh = o.headers;
    this.headers = (oh && oh._isHeaders) ? oh : new Headers(oh || (isReq ? input.headers : undefined));
    if (o.body !== undefined && o.body !== null) {
      if (typeof o.body === "string") {
        this._body = o.body;
        this._u8 = null;
        this._fd = null;
      } else {
        var nb = __normHttpBody(o.body, this.headers);
        this._body = nb.str;
        this._u8 = nb.u8;
        this._fd = nb.fd;
      }
    } else if (isReq) {
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
    var st = o.status !== undefined ? Number(o.status) : 200;
    this.status = st;
    this.statusText = o.statusText !== undefined ? String(o.statusText) : (st === 200 ? "OK" : "");
    this.ok = st >= 200 && st < 300;
    var oh = o.headers;
    this.headers = (oh && oh._isHeaders) ? oh : new Headers(oh);
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
  var LazyResProto = Object.create(SP);
  Object.defineProperty(LazyResProto, "headers", {
    get: function() {
      if (!this._h) {
        var h = new Headers();
        if (this._ct) h._m["content-type"] = this._ct;
        this._h = h;
      }
      return this._h;
    },
    set: function(v) {
      this._h = v;
    },
    configurable: true,
    enumerable: true
  });
  RP.text = SP.text = function() {
    this.bodyUsed = true;
    if (this._stream) {
      var st = this._stream, self = this;
      if (st._startPromise) {
        return st._startPromise.then(function() {
          self._body = st._toText();
          self._stream = null;
          return self._body;
        });
      }
      this._body = st._toText();
      this._stream = null;
    }
    if (this._data !== undefined && !this._body) this._body = JSON.stringify(this._data);
    return Promise.resolve(this._body);
  };
  RP.json = SP.json = function() {
    this.bodyUsed = true;
    if (this._data !== undefined) return Promise.resolve(this._data);
    try { return Promise.resolve(JSON.parse(this._body)); } catch (e) { return Promise.reject(e); }
  };
  RP.textSync = SP.textSync = function() {
    this.bodyUsed = true;
    if (this._data !== undefined && !this._body) this._body = JSON.stringify(this._data);
    return this._body;
  };
  RP.jsonSync = SP.jsonSync = function() {
    this.bodyUsed = true;
    if (this._data !== undefined) return this._data;
    return JSON.parse(this._body);
  };
  RP.arrayBuffer = SP.arrayBuffer = function() {
    this.bodyUsed = true;
    return Promise.resolve(__extractBodyBytes(this).buffer);
  };
  RP.bytes = SP.bytes = function() {
    this.bodyUsed = true;
    return Promise.resolve(__extractBodyBytes(this));
  };
  RP.blob = SP.blob = function() {
    this.bodyUsed = true;
    if (!globalThis.Blob) __slice_ensure_shard(15);
    return Promise.resolve(new globalThis.Blob([__extractBodyBytes(this)], { type: this._ct || this.headers.get("content-type") || "" }));
  };
  RP.formData = SP.formData = function() {
    this.bodyUsed = true;
    return Promise.resolve(__bodyToFormData(this));
  };
  RP.clone = function() {
    var r = new Request(this.url, {
      method: this.method,
      headers: new Headers(this.headers),
      body: this._body,
      redirect: this.redirect,
      signal: this.signal
    });
    if (this._u8) r._u8 = new Uint8Array(this._u8);
    if (this._fd) r._fd = this._fd;
    return r;
  };
  SP.clone = function() {
    if (this._data !== undefined && !this._body) this._body = JSON.stringify(this._data);
    var r = new Response(this._body, {
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
  Response.json = function(data, init) {
    if (!init) {
      var rFast = Object.create(LazyResProto);
      rFast.status = 200;
      rFast.statusText = "OK";
      rFast.ok = true;
      rFast._h = null;
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
    var r = new Response(null, typeof init === "number" ? { status: init } : init);
    if (!r.headers._m["content-type"]) r.headers._m["content-type"] = "application/json";
    r._data = data;
    return r;
  };
  Response.redirect = function(url, status) {
    var st = status || 302;
    return new Response("", { status: st, headers: { location: String(url) } });
  };
  Response.error = function() {
    var r = new Response("", { status: 0, statusText: "" });
    r.ok = false;
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

  function __finishSliceRes(r) {
    var h = r._h !== undefined ? r._h : r.headers;
    var ct = (h && h._m && h._m["content-type"]) || r._ct || "text/plain;charset=UTF-8";
    var hdrJson = h ? __resHdrJson(h) : "";
    if (r._stream) {
      var st = r._stream;
      if (st._startPromise) {
        st._startPromise.then(function() {
          var hs = r._h !== undefined ? r._h : r.headers;
          __slice_http_respond(r.status, st._toText(), ct, hs ? __resHdrJson(hs) : "");
        });
        return;
      }
      __slice_http_respond(r.status, st._toText(), ct, hdrJson);
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
    var res = fn.call(h, __makeLiveReq(), globalThis.process.env, __slice_web_ctx);
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
