// Slice (Nexus-JS) Prelude Shard 8: MicroVM Web Workers (`Worker`, `MessageChannel`, `MessagePort`)
// and Zero-Trust OCap TCP/TLS Sockets (`connect` / `Slice.connect` / `Slice.mockSocket`).
(function() {
  if (!globalThis.EventTarget) __slice_ensure_shard(13);
  if (!globalThis.TextEncoder) __slice_ensure_shard(2);

  function __serReplacer(_k, v) {
    var raw = (this && typeof this === "object") ? this[_k] : v;
    if (raw instanceof Date) {
      return { __slice_t: "date", t: raw.getTime() };
    }
    if (typeof raw === "number" && !Number.isFinite(raw)) {
      return { __slice_t: "num", v: raw !== raw ? "NaN" : (raw > 0 ? "Inf" : "-Inf") };
    }
    if (v instanceof Uint8Array) {
      return { __slice_t: "u8", a: Array.from(v) };
    }
    if (v instanceof ArrayBuffer) {
      return { __slice_t: "ab", a: Array.from(new Uint8Array(v)) };
    }
    if (typeof bigint === "string" || typeof v === "bigint") {
      return { __slice_t: "bi", s: v.toString() };
    }
    if (v instanceof Map) {
      return { __slice_t: "map", e: Array.from(v.entries()) };
    }
    if (v instanceof Set) {
      return { __slice_t: "set", v: Array.from(v.values()) };
    }
    if (v instanceof RegExp) {
      return { __slice_t: "re", s: v.source, f: v.flags };
    }
    if (v instanceof Error) {
      return { __slice_t: "err", n: v.name, m: v.message, st: v.stack || "" };
    }
    return v;
  }

  function __serReviver(_k, v) {
    if (v && typeof v === "object" && typeof v.__slice_t === "string") {
      var t = v.__slice_t;
      if (t === "date") return new Date(v.t === null ? NaN : v.t);
      if (t === "num") return v.v === "NaN" ? NaN : (v.v === "Inf" ? Infinity : -Infinity);
      if (t === "u8") return new Uint8Array(v.a || []);
      if (t === "ab") return new Uint8Array(v.a || []).buffer;
      if (t === "bi") return BigInt(v.s || "0");
      if (t === "map") return new Map(v.e || []);
      if (t === "set") return new Set(v.v || []);
      if (t === "re") return new RegExp(v.s || "", v.f || "");
      if (t === "err") {
        var e = new Error(v.m || "");
        e.name = v.n || "Error";
        if (v.st) e.stack = v.st;
        return e;
      }
      if (t === "undef") return undefined;
    }
    return v;
  }

  function __serializeMsg(val) {
    if (val === undefined) return "{\"__slice_t\":\"undef\"}";
    return JSON.stringify(val, __serReplacer);
  }

  function __deserializeMsg(jsonStr) {
    if (!jsonStr || jsonStr === "null") return null;
    return JSON.parse(jsonStr, __serReviver);
  }

  globalThis.__slice_worker_serialize = __serializeMsg;
  globalThis.__slice_worker_deserialize = __deserializeMsg;

  function __makeMsgEvent(data) {
    if (typeof globalThis.MessageEvent === "function") {
      return new globalThis.MessageEvent("message", { data: data });
    }
    var ev = new Event("message");
    ev.data = data;
    ev.origin = "";
    ev.lastEventId = "";
    ev.source = null;
    ev.ports = [];
    return ev;
  }

  // Dual EventTarget + Node EventEmitter helper mixin
  function __initNodeEmitter(obj, onListenerChanged) {
    obj._nodeEv = Object.create(null);
    obj._onLC = onListenerChanged || null;
  }

  function __attachNodeEmitterProto(Proto) {
    Proto.on = Proto.addListener = function(type, fn) {
      if (typeof fn !== "function") throw new TypeError("listener must be a function");
      var k = String(type);
      (this._nodeEv[k] || (this._nodeEv[k] = [])).push(fn);
      if (this._onLC) this._onLC(k);
      return this;
    };
    Proto.once = function(type, fn) {
      if (typeof fn !== "function") throw new TypeError("listener must be a function");
      var self = this;
      var k = String(type);
      function wrapped() {
        self.off(k, wrapped);
        fn.apply(self, arguments);
      }
      wrapped._orig = fn;
      return this.on(k, wrapped);
    };
    Proto.off = Proto.removeListener = function(type, fn) {
      var k = String(type);
      var list = this._nodeEv && this._nodeEv[k];
      if (!list) return this;
      var next = [];
      for (var i = 0; i < list.length; i++) {
        if (list[i] !== fn && list[i]._orig !== fn) next.push(list[i]);
      }
      if (next.length > 0) this._nodeEv[k] = next;
      else delete this._nodeEv[k];
      if (this._onLC) this._onLC(k);
      return this;
    };
    Proto.removeAllListeners = function(type) {
      if (arguments.length === 0) {
        this._nodeEv = Object.create(null);
      } else if (this._nodeEv) {
        delete this._nodeEv[String(type)];
      }
      if (this._onLC) this._onLC(type ? String(type) : "");
      return this;
    };
    Proto.listenerCount = function(type) {
      var k = String(type);
      var n = (this._nodeEv && this._nodeEv[k]) ? this._nodeEv[k].length : 0;
      var et = (this._l && this._l[k]) ? this._l[k].length : 0;
      var onProp = (typeof this["on" + k] === "function") ? 1 : 0;
      return n + et + onProp;
    };
    Proto.emit = function(type) {
      var k = String(type);
      var list = this._nodeEv && this._nodeEv[k];
      var args = [];
      for (var i = 1; i < arguments.length; i++) args.push(arguments[i]);
      if (list && list.length > 0) {
        var snap = list.slice();
        for (var j = 0; j < snap.length; j++) snap[j].apply(this, args);
        return true;
      }
      return false;
    };
  }

  // `MessagePort` & `MessageChannel` & `receiveMessageOnPort`
  function MessagePort() {
    EventTarget.call(this);
    this._entangled = null;
    this._closed = false;
    this._started = false;
    this._syncQueue = [];
    this._onmessage = null;
    this.onmessageerror = null;
    var self = this;
    __initNodeEmitter(this, function(evName) {
      if (evName === "message" && self.listenerCount("message") > 0) {
        self.start();
      }
    });
  }
  MessagePort.prototype = Object.create(EventTarget.prototype);
  MessagePort.prototype.constructor = MessagePort;
  __attachNodeEmitterProto(MessagePort.prototype);

  Object.defineProperty(MessagePort.prototype, "onmessage", {
    configurable: true,
    enumerable: true,
    get: function() { return this._onmessage; },
    set: function(fn) {
      this._onmessage = typeof fn === "function" ? fn : null;
      if (this._onmessage) this.start();
    }
  });

  MessagePort.prototype._hasAsyncListeners = function() {
    var hasNode = Boolean(this._nodeEv && this._nodeEv.message && this._nodeEv.message.length > 0);
    var hasEt = Boolean(this._started && this._l && this._l.message && this._l.message.length > 0);
    return Boolean(this._onmessage || hasNode || hasEt);
  };

  MessagePort.prototype._deliverOne = function(cloned) {
    if (this._closed) return;
    if (this._nodeEv && this._nodeEv.message && this._nodeEv.message.length > 0) {
      var snap = this._nodeEv.message.slice();
      for (var i = 0; i < snap.length; i++) snap[i].call(this, cloned);
    }
    var ev = __makeMsgEvent(cloned);
    if (typeof this._onmessage === "function") {
      this._onmessage.call(this, ev);
    }
    if (this._l && this._l.message && this._l.message.length > 0) {
      this.dispatchEvent(ev);
    }
  };

  MessagePort.prototype.start = function() {
    this._started = true;
    var self = this;
    if (this._syncQueue.length > 0 && this._hasAsyncListeners()) {
      queueMicrotask(function() {
        while (!self._closed && self._syncQueue.length > 0 && self._hasAsyncListeners()) {
          var item = self._syncQueue.shift();
          self._deliverOne(item);
        }
      });
    }
  };

  MessagePort.prototype.postMessage = function(value) {
    if (this._closed || !this._entangled || this._entangled._closed) return;
    var target = this._entangled;
    var cloned = __deserializeMsg(__serializeMsg(value));
    target._syncQueue.push(cloned);
    if (target._hasAsyncListeners()) {
      queueMicrotask(function() {
        if (!target._closed && target._syncQueue.length > 0 && target._hasAsyncListeners()) {
          var msg = target._syncQueue.shift();
          target._deliverOne(msg);
        }
      });
    }
  };

  MessagePort.prototype.close = function() {
    if (this._closed) return;
    this._closed = true;
    var peer = this._entangled;
    this._entangled = null;
    this.emit("close");
    this.dispatchEvent(new Event("close"));
    if (peer && !peer._closed) {
      peer._entangled = null;
    }
  };

  MessagePort.prototype.ref = function() { return this; };
  MessagePort.prototype.unref = function() { return this; };

  function MessageChannel() {
    this.port1 = new MessagePort();
    this.port2 = new MessagePort();
    this.port1._entangled = this.port2;
    this.port2._entangled = this.port1;
  }

  function receiveMessageOnPort(port) {
    if (!port || !(port instanceof MessagePort) || port._closed) return undefined;
    if (port._syncQueue.length === 0) return undefined;
    return { message: port._syncQueue.shift() };
  }

  globalThis.MessagePort = MessagePort;
  globalThis.MessageChannel = MessageChannel;
  globalThis.__slice_receiveMessageOnPort = receiveMessageOnPort;

  // Parent-side `Worker` registry and class
  var __activeWorkers = Object.create(null);

  function Worker(specifier, options) {
    if (!(this instanceof Worker)) return new Worker(specifier, options);
    EventTarget.call(this);
    __initNodeEmitter(this, null);
    var opts = (options && typeof options === "object") ? options : {};
    var specStr = (specifier && typeof specifier === "object" && specifier.href)
      ? String(specifier.href)
      : String(specifier || "");
    var isEval = Boolean(opts.eval);

    if (!isEval && specStr.indexOf("blob:slice/") === 0) {
      var blobs = globalThis.__slice_blob_urls;
      if (!blobs || typeof blobs[specStr] !== "string") {
        throw new Error("WorkerError: Unknown or revoked Blob URL '" + specStr + "'");
      }
      specStr = blobs[specStr];
      isEval = true;
    }

    var wDataJson = opts.workerData !== undefined ? __serializeMsg(opts.workerData) : "null";
    var spawnCfg = {
      eval: isEval,
      name: opts.name ? String(opts.name) : "",
      workerDataJson: wDataJson,
      slice: opts.slice || null
    };

    this.onmessage = null;
    this.onerror = null;
    this.onmessageerror = null;
    this._terminated = false;
    this._exited = false;
    this._exitCode = null;
    this._termResolvers = [];

    var wid = __slice_worker_spawn(specStr, JSON.stringify(spawnCfg));
    this.threadId = wid;
    __activeWorkers[wid] = this;
  }
  Worker.prototype = Object.create(EventTarget.prototype);
  Worker.prototype.constructor = Worker;
  __attachNodeEmitterProto(Worker.prototype);

  Worker.prototype.postMessage = function(value) {
    if (this._terminated || this._exited) return;
    var payload = __serializeMsg(value);
    __slice_worker_post_to_child(this.threadId, payload);
  };

  Worker.prototype.terminate = function() {
    var self = this;
    if (this._exited) {
      return Promise.resolve(this._exitCode !== null ? this._exitCode : 1);
    }
    this._terminated = true;
    __slice_worker_terminate_child(this.threadId);
    return new Promise(function(resolve) {
      self._termResolvers.push(resolve);
    });
  };

  Worker.prototype.ref = function() {
    __slice_worker_set_ref(this.threadId, true);
    return this;
  };

  Worker.prototype.unref = function() {
    __slice_worker_set_ref(this.threadId, false);
    return this;
  };

  globalThis.Worker = Worker;

  // Host -> Parent JS event dispatcher called by `workers.rs`
  globalThis.__slice_worker_host_deliver = function(workerId, evType, payloadStr, codeNum) {
    var w = __activeWorkers[workerId];
    if (!w || w._exited) return;
    if (evType === "online") {
      w.emit("online");
      w.dispatchEvent(new Event("online"));
      return;
    }
    if (evType === "message") {
      var data = __deserializeMsg(payloadStr);
      w.emit("message", data);
      var msgEv = __makeMsgEvent(data);
      w.dispatchEvent(msgEv);
      return;
    }
    if (evType === "error") {
      var errObj = new Error(payloadStr || "Worker error");
      if (w._nodeEv && w._nodeEv.error && w._nodeEv.error.length > 0) {
        w.emit("error", errObj);
      }
      var errEv = new Event("error");
      errEv.message = payloadStr || "Worker error";
      errEv.error = errObj;
      w.dispatchEvent(errEv);
      return;
    }
    if (evType === "exit") {
      w._exited = true;
      w._exitCode = Number(codeNum) || 0;
      delete __activeWorkers[workerId];
      w.emit("exit", w._exitCode);
      var exEv = new Event("exit");
      exEv.code = w._exitCode;
      w.dispatchEvent(exEv);
      if (w._termResolvers && w._termResolvers.length > 0) {
        var resolvers = w._termResolvers.slice();
        w._termResolvers.length = 0;
        for (var i = 0; i < resolvers.length; i++) {
          resolvers[i](w._exitCode);
        }
      }
    }
  };

  // Worker-side `parentPort`, `workerData`, `self.postMessage`, `self.onmessage`, `self.close`
  var __workerSelfPort = null;
  var __workerSelfMeta = null;

  function __syncWorkerSelfListening() {
    if (!__workerSelfPort) return;
    if (__workerSelfPort._closed || __workerSelfPort._unrefed) {
      __slice_worker_self_set_listening(false);
      return;
    }
    var count = __workerSelfPort.listenerCount("message") +
      (typeof globalThis._workerOnMessage === "function" ? 1 : 0);
    __slice_worker_self_set_listening(count > 0);
  }

  globalThis.__slice_init_worker_self_env = function(forceRefresh) {
    if (__workerSelfMeta && !forceRefresh) return __workerSelfMeta;
    var rawMeta = JSON.parse(__slice_worker_self_meta());
    var wData = rawMeta.isMainThread ? null : __deserializeMsg(rawMeta.workerDataJson);
    __workerSelfPort = null;
    globalThis._workerOnMessage = null;
    if (!rawMeta.isMainThread) {
      var port = new EventTarget();
      __initNodeEmitter(port, function() {
        __syncWorkerSelfListening();
      });
      __attachNodeEmitterProto(port);
      port._closed = false;
      port._unrefed = false;
      port._onmessage = null;
      Object.defineProperty(port, "onmessage", {
        configurable: true,
        enumerable: true,
        get: function() { return this._onmessage; },
        set: function(fn) {
          this._onmessage = typeof fn === "function" ? fn : null;
          __syncWorkerSelfListening();
        }
      });
      var origAddEL = port.addEventListener;
      port.addEventListener = function(type, fn, opts) {
        origAddEL.call(this, type, fn, opts);
        __syncWorkerSelfListening();
      };
      var origRemEL = port.removeEventListener;
      port.removeEventListener = function(type, fn, opts) {
        origRemEL.call(this, type, fn, opts);
        __syncWorkerSelfListening();
      };
      port.postMessage = function(val) {
        if (this._closed) return;
        __slice_worker_self_post(__serializeMsg(val));
      };
      port.close = function() {
        this._closed = true;
        __slice_worker_self_set_listening(false);
        this.emit("close");
        this.dispatchEvent(new Event("close"));
      };
      port.ref = function() {
        this._unrefed = false;
        __syncWorkerSelfListening();
        return this;
      };
      port.unref = function() {
        this._unrefed = true;
        __slice_worker_self_set_listening(false);
        return this;
      };
      port.start = function() {};
      __workerSelfPort = port;

      globalThis.self = globalThis;
      globalThis.name = rawMeta.name || "";
      globalThis.postMessage = function(val) {
        port.postMessage(val);
      };
      globalThis.close = function() {
        port.close();
        __slice_worker_self_set_listening(false, 0);
      };
      Object.defineProperty(globalThis, "onmessage", {
        configurable: true,
        enumerable: true,
        get: function() { return globalThis._workerOnMessage || null; },
        set: function(fn) {
          globalThis._workerOnMessage = typeof fn === "function" ? fn : null;
          __syncWorkerSelfListening();
        }
      });
      globalThis.addEventListener = function(type, fn, opts) {
        port.addEventListener(type, fn, opts);
      };
      globalThis.removeEventListener = function(type, fn, opts) {
        port.removeEventListener(type, fn, opts);
      };
    } else {
      globalThis.self = globalThis;
    }

    __workerSelfMeta = {
      isMainThread: Boolean(rawMeta.isMainThread),
      threadId: Number(rawMeta.threadId) || 0,
      workerData: wData,
      parentPort: __workerSelfPort
    };
    return __workerSelfMeta;
  };

  globalThis.__slice_worker_self_deliver = function(msgJson) {
    if (!__workerSelfPort || __workerSelfPort._closed) return;
    var data = __deserializeMsg(msgJson);
    __workerSelfPort.emit("message", data);
    var ev = __makeMsgEvent(data);
    if (typeof globalThis._workerOnMessage === "function") {
      globalThis._workerOnMessage.call(globalThis, ev);
    }
    __workerSelfPort.dispatchEvent(ev);
    __syncWorkerSelfListening();
  };

  // WinterTC `connect(address, options)` (`cloudflare:sockets` & `Slice.connect`)
  function __b64ToU8(b64) {
    if (!b64) return new Uint8Array(0);
    var bin = atob(b64);
    var out = new Uint8Array(bin.length);
    for (var i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i) & 0xff;
    return out;
  }
  globalThis.__slice_b64ToU8 = __b64ToU8;

  function __parseSocketAddress(address) {
    if (typeof address === "string") {
      var trimmed = address.trim();
      var colonIdx = trimmed.lastIndexOf(":");
      if (colonIdx <= 0) {
        throw new TypeError("Invalid socket address '" + address + "': expected 'hostname:port'");
      }
      var host = trimmed.slice(0, colonIdx);
      if (host.charCodeAt(0) === 91 && host.charCodeAt(host.length - 1) === 93) {
        host = host.slice(1, -1);
      }
      var port = Number(trimmed.slice(colonIdx + 1));
      if (!Number.isInteger(port) || port <= 0 || port > 65535) {
        throw new TypeError("Invalid socket port in '" + address + "'");
      }
      return { hostname: host, port: port };
    }
    if (address && typeof address === "object") {
      var h = String(address.hostname || address.host || "127.0.0.1");
      var p = Number(address.port);
      if (!Number.isInteger(p) || p <= 0 || p > 65535) {
        throw new TypeError("Invalid socket port: " + address.port);
      }
      return { hostname: h, port: p };
    }
    throw new TypeError("connect(address) requires a 'host:port' string or { hostname, port } object");
  }

  function __makeWinterTcSocket(sockId, meta, origHostname, secureTransportMode) {
    if (!globalThis.ReadableStream) __slice_ensure_shard(3);
    if (!globalThis.WritableStream) __slice_ensure_shard(14);
    var closedResolved = false;
    var resolveClosed, rejectClosed;
    var closedPromise = new Promise(function(res, rej) {
      resolveClosed = res;
      rejectClosed = rej;
    });

    function markClosed() {
      if (!closedResolved) {
        closedResolved = true;
        __slice_tcp_close(sockId);
        resolveClosed(undefined);
      }
    }

    var readable = new ReadableStream({
      pull: function(controller) {
        if (closedResolved) {
          controller.close();
          return;
        }
        var rawRes = JSON.parse(__slice_tcp_read(sockId, 65536, 200));
        if (rawRes.bytesRead > 0 && rawRes.dataBase64) {
          controller.enqueue(__b64ToU8(rawRes.dataBase64));
        }
        if (rawRes.eof) {
          markClosed();
          controller.close();
        }
      },
      cancel: function() {
        markClosed();
      }
    });

    var writable = new WritableStream({
      write: function(chunk) {
        if (closedResolved) {
          throw new Error("SocketError: Cannot write to a closed socket");
        }
        var u8 = __slice_toU8(chunk);
        __slice_tcp_write(sockId, u8);
      },
      close: function() {
        markClosed();
      },
      abort: function() {
        markClosed();
      }
    });

    var remStr = meta.remotePort
      ? (meta.remoteAddress + ":" + meta.remotePort)
      : (meta.remoteAddress || (origHostname + ":" + meta.port));
    var locStr = meta.localPort
      ? (meta.localAddress + ":" + meta.localPort)
      : (meta.localAddress || "127.0.0.1:0");
    var openedPromise = Promise.resolve({
      remoteAddress: remStr,
      localAddress: locStr
    });

    return {
      readable: readable,
      writable: writable,
      opened: openedPromise,
      closed: closedPromise,
      secureTransport: secureTransportMode,
      close: function() {
        markClosed();
        return closedPromise;
      },
      startTls: function(tlsOpts) {
        if (secureTransportMode !== "starttls") {
          throw new Error("SocketError: startTls() can only be called when secureTransport is 'starttls'");
        }
        var sniHost = (tlsOpts && tlsOpts.expectedServerHostname)
          ? String(tlsOpts.expectedServerHostname)
          : origHostname;
        var upgradedJson = __slice_tcp_starttls(sockId, sniHost);
        var upgradedMeta = JSON.parse(upgradedJson);
        return __makeWinterTcSocket(sockId, upgradedMeta, sniHost, "on");
      }
    };
  }

  function connect(address, options) {
    var addr = __parseSocketAddress(address);
    var opts = (options && typeof options === "object") ? options : {};
    var secMode = opts.secureTransport ? String(opts.secureTransport) : "off";
    if (secMode !== "off" && secMode !== "on" && secMode !== "starttls") {
      throw new TypeError("Invalid secureTransport option: " + secMode);
    }
    var metaJson = __slice_tcp_connect(addr.hostname, addr.port, secMode);
    var meta = JSON.parse(metaJson);
    return __makeWinterTcSocket(meta.id, meta, addr.hostname, secMode);
  }

  globalThis.__slice_wintertc_connect = connect;
  globalThis.Slice.connect = connect;
  globalThis.Slice.mockSocket = function(targetAddr, specOrBytes) {
    if (typeof specOrBytes === "string") {
      if (
        specOrBytes === "echo" ||
        specOrBytes === "redis" ||
        specOrBytes.indexOf("prefix:") === 0 ||
        specOrBytes.indexOf("reply:") === 0 ||
        specOrBytes.indexOf("banner:") === 0
      ) {
        __slice_socket_mock(String(targetAddr), specOrBytes);
      } else {
        __slice_socket_mock(String(targetAddr), JSON.stringify({ responseText: specOrBytes }));
      }
      return;
    }
    if (specOrBytes instanceof Uint8Array) {
      __slice_socket_mock(
        String(targetAddr),
        JSON.stringify({ responseBytes: Array.from(specOrBytes) })
      );
      return;
    }
    __slice_socket_mock(String(targetAddr), JSON.stringify(specOrBytes || {}));
  };
})();
