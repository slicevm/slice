  function Request(input, init) {
    if (input === "__SLICE_LIVE_REQ__") {
      return __makeLiveReq();
    }
    var fastUrl = typeof input === "string"
      ? input
      : (input && input._isSliceUrl === true ? input.href : null);
    if (!init && fastUrl !== null) {
      return new __FastReq(fastUrl, "GET", null, "", "follow", __defaultReqSignal);
    }
    var o = init || {};
    var oh = o.headers;
    if (
      fastUrl !== null &&
      (o.body === undefined || o.body === null || typeof o.body === "string") &&
      (!oh || (typeof oh === "object" && !Array.isArray(oh) && (oh._isHeaders || !(__symIter && typeof oh[__symIter] === "function"))))
    ) {
      var rawMFast = o.method;
      var fastMethod = rawMFast
        ? (rawMFast === "GET" || rawMFast === "POST" ? rawMFast : String(rawMFast).toUpperCase())
        : "GET";
      if (o.body !== undefined && o.body !== null && (fastMethod === "GET" || fastMethod === "HEAD")) {
        throw new TypeError("Request with GET/HEAD method cannot have body");
      }
      var isStrReqBody = typeof o.body === "string";
      var rInit = new __FastReq(
        fastUrl,
        fastMethod,
        isStrReqBody ? o.body : null,
        "",
        o.redirect ? String(o.redirect) : "follow",
        o.signal || __defaultReqSignal
      );
      if (oh) {
        __assignHeadersMap(rInit, "_hm", "_hsc", oh);
      }
      if (isStrReqBody && (!rInit._hm || rInit._hm["content-type"] === undefined)) {
        rInit._ct = "text/plain;charset=UTF-8";
      }
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
    this._h = (oh && oh._isHeaders) ? new Headers(oh) : new Headers(oh || (isReq ? input.headers : undefined));
    this._hm = null;
    this._hsc = null;
    this._ct = "";
    this._stream = null;
    if (o.body !== undefined && o.body !== null) {
      if (this.method === "GET" || this.method === "HEAD") {
        throw new TypeError("Request with GET/HEAD method cannot have body");
      }
      if (typeof o.body === "string") {
        this._body = o.body;
        this._u8 = null;
        if (!this._h._m["content-type"]) {
          this._h._m["content-type"] = "text/plain;charset=UTF-8";
        }
      } else if (__isReadableStreamObj(o.body)) {
        __ensureUsableStream(o.body);
        this._stream = o.body;
        this._body = null;
        this._u8 = null;
      } else {
        var nb = __normHttpBody(o.body, this._h);
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
        var isSimpleStr = typeof body === "string";
        return new __FastRes(
          200,
          "OK",
          isSimpleStr ? body : null,
          isSimpleStr ? "text/plain;charset=UTF-8" : ""
        );
      }
      this.status = 200;
      this.statusText = "OK";
      this.ok = true;
      this._h = new Headers();
      this._hm = null;
      this._hsc = null;
      this._stream = null;
      if (__isReadableStreamObj(body)) {
        __ensureUsableStream(body);
        this._stream = body;
        this._body = null;
        this._u8 = null;
      } else {
        var nbFast = __normHttpBody(body, this._h);
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
      (!oh || (typeof oh === "object" && !Array.isArray(oh) && (oh._isHeaders || !(__symIter && typeof oh[__symIter] === "function"))))
    ) {
      var isInitStr = typeof body === "string";
      var rInit = new __FastRes(st, stText, isInitStr ? body : null, "");
      if (oh) {
        __assignHeadersMap(rInit, "_hm", "_hsc", oh);
      }
      if (isInitStr && (!rInit._hm || rInit._hm["content-type"] === undefined)) {
        rInit._ct = "text/plain;charset=UTF-8";
      }
      return rInit;
    }
    this.status = st;
    this.statusText = stText;
    this.ok = st >= 200 && st < 300;
    this._h = (oh && oh._isHeaders) ? new Headers(oh) : new Headers(oh);
    this._hm = null;
    this._hsc = null;
    this._stream = null;
    this._ct = "";
    if (body === undefined || body === null) {
      this._body = null;
      this._u8 = null;
    } else if (typeof body === "string") {
      this._body = body;
      this._u8 = null;
      if (!this._h._m["content-type"]) {
        this._h._m["content-type"] = "text/plain;charset=UTF-8";
      }
    } else if (__isReadableStreamObj(body)) {
      __ensureUsableStream(body);
      this._stream = body;
      this._body = null;
      this._u8 = null;
    } else {
      var nb = __normHttpBody(body, this._h);
      this._body = nb.str;
      this._u8 = nb.u8;
    }
    this._isSliceRes = true;
    this._used = false;
  }

  var RP = Request.prototype, SP = Response.prototype;
  __FastReq.prototype = RP;
  __FastRes.prototype = SP;
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
  var __lazyHeadersDesc = {
    get: function() {
      if (!this._h) {
        var hm = this._hm || Object.create(null);
        if (this._ct && hm["content-type"] === undefined) hm["content-type"] = this._ct;
        var h = new __FastHdr(hm, this._hsc || null);
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
  };
  Object.defineProperty(RP, "headers", __lazyHeadersDesc);
  Object.defineProperty(SP, "headers", __lazyHeadersDesc);

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
      var st = this._stream;
      if ((st._closed || st._closeRequested) && !st._errored && !st._startPromise && !st._pulling) {
        st.getReader();
        var txt;
        if (__streamHasBinary(st) && typeof st._toBytes === "function") {
          this._u8 = st._toBytes();
          this._body = null;
          txt = __getBodyText(this);
        } else {
          txt = st._toText();
          this._body = txt;
        }
        return Promise.resolve(txt);
      }
      return st._drainToBody(this).then(__getBodyText);
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
      var st = this._stream;
      if ((st._closed || st._closeRequested) && !st._errored && !st._startPromise && !st._pulling) {
        st.getReader();
        var txt;
        if (__streamHasBinary(st) && typeof st._toBytes === "function") {
          this._u8 = st._toBytes();
          this._body = null;
          txt = __getBodyText(this);
        } else {
          txt = st._toText();
          this._body = txt;
        }
        try {
          return Promise.resolve(JSON.parse(txt));
        } catch (eJson) {
          return Promise.reject(eJson);
        }
      }
      return st._drainToBody(this).then(function(s) {
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
    var r = new __FastRes(status, statusText || "", null, "");
    if (rawHeaders) {
      __assignHeadersMap(r, "_hm", "_hsc", rawHeaders);
    }
    return r;
  }
  globalThis.__slice_make_special_response = __makeSpecialResponse;

  Response.json = function(data, init) {
    var jsonStr = JSON.stringify(data);
    if (jsonStr === undefined) {
      throw new TypeError("Value is not JSON serializable");
    }
    if (!init) {
      return new __FastRes(200, "OK", jsonStr, "application/json");
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
        (!jOh || (typeof jOh === "object" && !Array.isArray(jOh) && (jOh._isHeaders || !(__symIter && typeof jOh[__symIter] === "function"))))
      ) {
        var rJson = new __FastRes(
          jSt,
          init.statusText !== undefined ? String(init.statusText) : (jSt === 200 ? "OK" : ""),
          jsonStr,
          "application/json"
        );
        if (jOh) {
          __assignHeadersMap(rJson, "_hm", "_hsc", jOh);
          if (rJson._hm["content-type"] !== undefined) rJson._ct = "";
        }
        return rJson;
      }
    }
    var r = new Response(null, typeof init === "number" ? { status: init } : init);
    r._body = jsonStr;
    if (r._h === null) {
      if (!r._hm || r._hm["content-type"] === undefined) r._ct = "application/json";
    } else if (!r._h._m["content-type"]) {
      r._h._m["content-type"] = "application/json";
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
