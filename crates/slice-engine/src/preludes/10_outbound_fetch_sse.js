// Slice (Nexus-JS) Lazy Prelude Shard 10: Outbound Fetch, AbortSignal/AbortController & SSE (`10_outbound_fetch_sse.js`)
(function() {
  if (!globalThis.DOMException) __slice_ensure_shard(13);
  function __mkAbortErr() {
    return new DOMException("The operation was aborted.", "AbortError");
  }
  function __mkTimeoutErr() {
    return new DOMException("TimeoutError: The operation was aborted due to timeout", "TimeoutError");
  }

  function AbortSignal() {
    this.aborted = false;
    this.reason = undefined;
    this.onabort = null;
    this._ls = [];
  }
  AbortSignal.prototype.addEventListener = function(t, fn) {
    if (t === "abort" && typeof fn === "function") this._ls.push(fn);
  };
  AbortSignal.prototype.removeEventListener = function(t, fn) {
    if (t === "abort") {
      var nx = [];
      for (var i = 0; i < this._ls.length; i++) {
        if (this._ls[i] !== fn) nx.push(this._ls[i]);
      }
      this._ls = nx;
    }
  };
  AbortSignal.prototype.throwIfAborted = function() {
    if (this.aborted) throw this.reason;
  };
  AbortSignal.prototype._fire = function(r) {
    if (this.aborted) return;
    this.aborted = true;
    this.reason = r !== undefined ? r : __mkAbortErr();
    var ev = { type: "abort", target: this };
    if (typeof this.onabort === "function") {
      try { this.onabort(ev); } catch (_e) {}
    }
    var ls = this._ls.slice();
    this._ls = [];
    for (var i = 0; i < ls.length; i++) {
      try { ls[i](ev); } catch (_e2) {}
    }
  };
  AbortSignal.prototype.dispatchEvent = function(ev) {
    if (ev && ev.type === "abort") {
      this._fire(this.reason);
      return true;
    }
    return true;
  };
  AbortSignal.abort = function(r) {
    var s = new AbortSignal();
    s._fire(r !== undefined ? r : __mkAbortErr());
    return s;
  };
  AbortSignal.timeout = function(ms) {
    var s = new AbortSignal();
    setTimeout(function() {
      s._fire(__mkTimeoutErr());
    }, Number(ms) || 0);
    return s;
  };
  AbortSignal.any = function(signals) {
    if (!signals || typeof signals.length !== "number") {
      throw new TypeError("AbortSignal.any requires an array or iterable of AbortSignal instances");
    }
    var s = new AbortSignal();
    for (var i = 0; i < signals.length; i++) {
      var sig = signals[i];
      if (sig && sig.aborted) {
        s._fire(sig.reason);
        return s;
      }
    }
    for (var j = 0; j < signals.length; j++) {
      (function(sig2) {
        if (sig2 && typeof sig2.addEventListener === "function") {
          sig2.addEventListener("abort", function() {
            s._fire(sig2.reason);
          });
        }
      })(signals[j]);
    }
    return s;
  };

  function AbortController() {
    this.signal = new AbortSignal();
  }
  AbortController.prototype.abort = function(r) {
    this.signal._fire(r);
  };

  globalThis.AbortSignal = AbortSignal;
  globalThis.AbortController = AbortController;

  function __fmtSse(ev) {
    if (ev === undefined || ev === null) return "";
    if (typeof ev === "string") {
      if (ev.indexOf("\n") === -1) return "data: " + ev + "\n\n";
      var ls = ev.split("\n"), o = "";
      for (var i = 0; i < ls.length; i++) o += "data: " + ls[i] + "\n";
      return o + "\n";
    }
    var s = "";
    if (ev.id !== undefined && ev.id !== null) s += "id: " + String(ev.id) + "\n";
    if (ev.event !== undefined && ev.event !== null) s += "event: " + String(ev.event) + "\n";
    if (ev.retry !== undefined && ev.retry !== null) s += "retry: " + Number(ev.retry) + "\n";
    var d = ev.data !== undefined ? (typeof ev.data === "string" ? ev.data : JSON.stringify(ev.data)) : JSON.stringify(ev);
    if (d.indexOf("\n") === -1) return s + "data: " + d + "\n\n";
    var dl = d.split("\n");
    for (var j = 0; j < dl.length; j++) s += "data: " + dl[j] + "\n";
    return s + "\n";
  }

  function __colSse(src) {
    var out = "";
    if (Array.isArray(src)) {
      for (var i = 0; i < src.length; i++) out += __fmtSse(src[i]);
    } else if (typeof src === "function") {
      var r = src(function(ev) { out += __fmtSse(ev); });
      if (r && typeof r.next === "function") {
        var st = r.next();
        while (!st.done) {
          out += __fmtSse(st.value);
          st = r.next();
        }
      } else if (Array.isArray(r)) {
        for (var j = 0; j < r.length; j++) out += __fmtSse(r[j]);
      }
    } else if (src && typeof src.next === "function") {
      var s2 = src.next();
      while (!s2.done) {
        out += __fmtSse(s2.value);
        s2 = src.next();
      }
    } else if (src !== undefined && src !== null) {
      out += __fmtSse(src);
    }
    return out;
  }

  if (!globalThis.Response) __slice_ensure_shard(11);
  var Response = globalThis.Response;
  var Headers = globalThis.Headers;
  var Request = globalThis.Request;

  if (Response) {
    Response.sse = function(src, init) {
      var o = init ? (typeof init === "number" ? { status: init } : init) : {};
      var h = o.headers instanceof Headers ? o.headers : new Headers(o.headers);
      if (!h._m["content-type"]) h._m["content-type"] = "text/event-stream; charset=utf-8";
      if (!h._m["cache-control"]) h._m["cache-control"] = "no-cache";
      var r = new Response(__colSse(src), { status: o.status || 200, statusText: o.statusText, headers: h });
      r._ct = h._m["content-type"];
      return r;
    };
  }

  function __wireToResponse(raw, fallbackUrl) {
    var bodyArg = raw.bodyBase64 ? __slice_base64_decode_bytes(raw.bodyBase64) : raw.body;
    var r = new Response(bodyArg, { status: raw.status, statusText: raw.statusText, headers: raw.headers });
    r.url = raw.finalUrl || fallbackUrl;
    r.redirected = Boolean(raw.redirected);
    return r;
  }

  function __reqHeadersJson(req) {
    if (req._h === null && req._hm === null) return "[]";
    return JSON.stringify(req.headers.entries());
  }

  function __doFetch(input, init) {
    var req = new Request(input, init);
    var sig = (init && init.signal) || req.signal || null;
    if (sig && sig.aborted) throw (sig.reason !== undefined ? sig.reason : __mkAbortErr());
    var reqBody = req._u8 instanceof Uint8Array ? req._u8 : req._body;
    var raw = JSON.parse(
      __slice_fetch_sync(req.url, req.method, __reqHeadersJson(req), reqBody, req.redirect || "follow")
    );
    if (raw.__error) throw new Error(raw.__error);
    return __wireToResponse(raw, req.url);
  }

  globalThis.fetch = function(input, init) {
    try {
      var req = new Request(input, init);
      var sig = (init && init.signal) || req.signal || null;
      if (sig && sig.aborted) {
        return Promise.reject(sig.reason !== undefined ? sig.reason : __mkAbortErr());
      }
      var dispatchFetch = function(reqBodyVal) {
        var hdrs = __reqHeadersJson(req);
        var fast = __slice_fetch_try_fast(req.url, req.method, hdrs, reqBodyVal, req.redirect || "follow");
        if (fast !== "") {
          var raw = JSON.parse(fast);
          if (raw.__error) return Promise.reject(new Error(raw.__error));
          return Promise.resolve(__wireToResponse(raw, req.url));
        }
        return new Promise(function(res, rej) {
          var settled = false;
          if (sig && typeof sig.addEventListener === "function") {
            sig.addEventListener("abort", function() {
              if (!settled) {
                settled = true;
                rej(sig.reason !== undefined ? sig.reason : __mkAbortErr());
              }
            });
          }
          __slice_fetch_enqueue(
            req.url,
            req.method,
            hdrs,
            reqBodyVal,
            req.redirect || "follow",
            function(rawStr) {
              if (settled) return;
              settled = true;
              var raw2 = JSON.parse(rawStr);
              if (raw2.__error) {
                rej(new Error(raw2.__error));
                return;
              }
              res(__wireToResponse(raw2, req.url));
            },
            function(errReason) {
              if (settled) return;
              settled = true;
              rej(errReason !== undefined ? errReason : __mkAbortErr());
            },
            sig
          );
        });
      };
      if (req._stream) {
        return req.bytes().then(dispatchFetch);
      }
      return dispatchFetch(req._u8 instanceof Uint8Array ? req._u8 : req._body);
    } catch (e) {
      return Promise.reject(e);
    }
  };

  if (globalThis.Slice) {
    globalThis.Slice.streamSSE = function(src, init) {
      var r = Response.sse(src, init);
      __slice_http_respond(r.status, r._body, r._ct, "");
      return r;
    };
    globalThis.Slice.fetchSync = __doFetch;
    globalThis.Slice.mockFetch = function(a, b, c) {
      var m = "*", u = String(a), o = b || {};
      if (c !== undefined) {
        m = String(a).toUpperCase();
        u = String(b);
        o = c || {};
      }
      var h = new Headers(o.headers);
      var isBin = o.body instanceof Uint8Array || o.body instanceof ArrayBuffer || Boolean(o.body && o.body.buffer instanceof ArrayBuffer);
      var wireObj = {
        status: o.status || 200,
        statusText: o.statusText || "OK",
        headers: h.entries(),
        body: ""
      };
      if (isBin) {
        wireObj.bodyBase64 = __slice_base64_encode_bytes(__slice_toU8(o.body), false);
      } else {
        wireObj.body = o.body === undefined || o.body === null ? "" : (typeof o.body === "string" ? o.body : JSON.stringify(o.body));
        if (typeof o.body === "object" && o.body !== null && !h._m["content-type"]) {
          h._m["content-type"] = "application/json";
          wireObj.headers = h.entries();
        }
      }
      __slice_mock_fetch(
        m === "*" ? u : (m + " " + u),
        JSON.stringify(wireObj)
      );
    };
  }
})();

