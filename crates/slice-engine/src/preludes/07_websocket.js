// Slice (Nexus-JS) Lazy Prelude Shard 7: RFC 6455 `WebSocket`, `MessageEvent`, `CloseEvent`, Server Upgrade (`07_websocket.js`)
(function() {
  if (!globalThis.EventTarget) __slice_ensure_shard(13);
  var _Event = globalThis.Event;
  var _EventTarget = globalThis.EventTarget;

  function MessageEvent(type, init) {
    _Event.call(this, type, init);
    var o = init || {};
    this.data = o.data !== undefined ? o.data : null;
    this.origin = o.origin !== undefined ? String(o.origin) : "";
    this.lastEventId = o.lastEventId !== undefined ? String(o.lastEventId) : "";
    this.source = o.source || null;
    this.ports = Array.isArray(o.ports) ? o.ports : [];
  }
  MessageEvent.prototype = Object.create(_Event.prototype);
  MessageEvent.prototype.constructor = MessageEvent;
  globalThis.MessageEvent = MessageEvent;

  function CloseEvent(type, init) {
    _Event.call(this, type, init);
    var o = init || {};
    this.wasClean = Boolean(o.wasClean !== undefined ? o.wasClean : true);
    this.code = o.code !== undefined ? Number(o.code) | 0 : 1000;
    this.reason = o.reason !== undefined ? String(o.reason) : "";
  }
  CloseEvent.prototype = Object.create(_Event.prototype);
  CloseEvent.prototype.constructor = CloseEvent;
  globalThis.CloseEvent = CloseEvent;

  var __wsJsMocks = Object.create(null);

  function __dispatchWsWireEvents(ws, rawJson) {
    if (!rawJson) return;
    var list;
    try { list = JSON.parse(rawJson); } catch (_e) { return; }
    if (!Array.isArray(list)) return;
    for (var i = 0; i < list.length; i++) {
      var ev = list[i];
      if (ev.eventType === "message") {
        var msgData = ev.data;
        if (ev.isBinary) {
          var u8 = __slice_base64_decode_bytes(ev.data);
          if (ws.binaryType === "arraybuffer") {
            msgData = u8.buffer.slice(u8.byteOffset, u8.byteOffset + u8.byteLength);
          } else if (ws.binaryType === "uint8array") {
            msgData = u8;
          } else {
            if (!globalThis.Blob) __slice_ensure_shard(15);
            msgData = new globalThis.Blob([u8]);
          }
        }
        var mEvt = new MessageEvent("message", { data: msgData, origin: ws.url });
        ws.dispatchEvent(mEvt);
      } else if (ev.eventType === "close") {
        ws.readyState = 3;
        var cEvt = new CloseEvent("close", {
          wasClean: ev.code === 1000,
          code: ev.code || 1000,
          reason: ev.reason || ""
        });
        ws.dispatchEvent(cEvt);
      }
    }
  }

  function WebSocket(url, protocols) {
    _EventTarget.call(this);
    if (arguments.length === 0) {
      throw new TypeError("Failed to construct 'WebSocket': 1 argument required");
    }
    this.url = String(url);
    this.readyState = 0;
    this.bufferedAmount = 0;
    this.extensions = "";
    this.binaryType = "blob";
    this.onopen = null;
    this._onmessage = null;
    this.onerror = null;
    this.onclose = null;
    var protoStr = "";
    if (Array.isArray(protocols)) protoStr = protocols.join(", ");
    else if (protocols !== undefined && protocols !== null) protoStr = String(protocols);

    var infoJson = __slice_ws_connect(this.url, protoStr);
    var info = JSON.parse(infoJson);
    this._id = info.id;
    this._isLive = Boolean(info.live);
    this._pollTimer = null;
    this.protocol = info.protocol || protoStr;
    this._jsMockFn = null;
    for (var pat in __wsJsMocks) {
      if (pat === "*" || this.url === pat || this.url.indexOf(pat) === 0) {
        this._jsMockFn = __wsJsMocks[pat];
        break;
      }
    }
    var self = this;
    var initialEvents = info.events && info.events.length > 0 ? JSON.stringify(info.events) : "";
    queueMicrotask(function() {
      if (self.readyState === 0) {
        self.readyState = 1;
        var oEvt = new _Event("open");
        self.dispatchEvent(oEvt);
        if (initialEvents) {
          __dispatchWsWireEvents(self, initialEvents);
        } else if (self._isLive && self.readyState === 1) {
          self._schedulePoll();
        }
      }
    });
  }
  WebSocket.prototype = Object.create(_EventTarget.prototype);
  WebSocket.prototype.constructor = WebSocket;
  WebSocket.CONNECTING = 0;
  WebSocket.OPEN = 1;
  WebSocket.CLOSING = 2;
  WebSocket.CLOSED = 3;
  WebSocket.prototype.CONNECTING = 0;
  WebSocket.prototype.OPEN = 1;
  WebSocket.prototype.CLOSING = 2;
  WebSocket.prototype.CLOSED = 3;

  Object.defineProperty(WebSocket.prototype, "onmessage", {
    get: function() {
      return this._onmessage;
    },
    set: function(fn) {
      this._onmessage = typeof fn === "function" ? fn : null;
      if (this._onmessage && this.readyState === 1) {
        this._schedulePoll();
      }
    },
    configurable: true,
    enumerable: true
  });

  WebSocket.prototype.addEventListener = function(type, listener, options) {
    _EventTarget.prototype.addEventListener.call(this, type, listener, options);
    if (String(type) === "message" && this.readyState === 1) {
      this._schedulePoll();
    }
  };

  WebSocket.prototype._hasMsgListeners = function() {
    return typeof this._onmessage === "function"
      || !!(this._et && this._et.message && this._et.message.length > 0);
  };

  WebSocket.prototype._schedulePoll = function() {
    if (!this._isLive || this._pollTimer !== null || this.readyState !== 1 || !this._hasMsgListeners()) {
      return;
    }
    var self = this;
    var remaining = 250;
    var emptyTicks = 0;
    function step() {
      self._pollTimer = null;
      if (self.readyState !== 1 || !self._hasMsgListeners()) return;
      var eventsJson = __slice_ws_poll(self._id);
      if (eventsJson && eventsJson !== "[]") {
        __dispatchWsWireEvents(self, eventsJson);
        return;
      }
      if (--remaining > 0 && self.readyState === 1 && self._hasMsgListeners()) {
        emptyTicks++;
        self._pollTimer = setTimeout(step, emptyTicks === 1 ? 1 : 2);
      }
    }
    this._pollTimer = setTimeout(step, 0);
  };

  WebSocket.prototype.send = function(data) {
    if (this.readyState === 2 || this.readyState === 3) {
      throw new Error("InvalidStateError: WebSocket is already in CLOSING or CLOSED state");
    }
    if (this.readyState === 0) {
      this.readyState = 1;
    }
    var isBinary = false;
    var wireStr = "";
    if (typeof data === "string") {
      wireStr = data;
    } else {
      isBinary = true;
      var u8 = __slice_toU8(data);
      wireStr = __slice_base64_encode_bytes(u8, false);
    }
    var eventsJson = __slice_ws_send(this._id, isBinary, wireStr);
    var self = this;
    if (this._jsMockFn) {
      var customReply = this._jsMockFn(data, this);
      if (customReply !== undefined) {
        queueMicrotask(function() {
          if (self.readyState !== 1) return;
          var mEvt = new MessageEvent("message", { data: customReply, origin: self.url });
          self.dispatchEvent(mEvt);
        });
        return;
      }
    }
    if (eventsJson && eventsJson !== "[]") {
      queueMicrotask(function() {
        __dispatchWsWireEvents(self, eventsJson);
      });
    } else if (this._isLive) {
      this._schedulePoll();
    }
  };

  WebSocket.prototype.poll = function() {
    if (this.readyState >= 2) return;
    var eventsJson = __slice_ws_poll(this._id);
    if (eventsJson && eventsJson !== "[]") {
      __dispatchWsWireEvents(this, eventsJson);
    }
  };

  WebSocket.prototype.close = function(code, reason) {
    if (this.readyState === 2 || this.readyState === 3) return;
    if (this._pollTimer !== null) {
      clearTimeout(this._pollTimer);
      this._pollTimer = null;
    }
    var c = code !== undefined ? Number(code) | 0 : 1000;
    var r = reason !== undefined ? String(reason) : "";
    this.readyState = 2;
    var eventsJson = __slice_ws_close(this._id, c, r);
    var self = this;
    queueMicrotask(function() {
      __dispatchWsWireEvents(self, eventsJson);
    });
  };
  globalThis.WebSocket = WebSocket;

  if (globalThis.Slice) {
    globalThis.Slice.mockWebSocket = function(urlPattern, handlerOrSpec) {
      var pat = String(urlPattern);
      if (typeof handlerOrSpec === "function") {
        __wsJsMocks[pat] = handlerOrSpec;
        return __slice_ws_mock(pat, "echo");
      }
      delete __wsJsMocks[pat];
      return __slice_ws_mock(pat, handlerOrSpec !== undefined ? String(handlerOrSpec) : "echo");
    };
    globalThis.Slice.onWebSocket = function(handler) {
      globalThis.__slice_ws_server_handler = handler;
    };
    globalThis.Slice.upgradeWebSocket = function(handlerOrOpts) {
      if (handlerOrOpts) {
        globalThis.__slice_ws_server_handler = handlerOrOpts;
      }
      if (typeof __slice_mark_ws_upgrade === "function") {
        __slice_mark_ws_upgrade();
      }
      if (!globalThis.Response) __slice_ensure_shard(11);
      __slice_http_respond(
        101,
        "",
        "",
        JSON.stringify([["Upgrade", "websocket"], ["Connection", "Upgrade"]])
      );
      return globalThis.__slice_make_special_response(101, "Switching Protocols", {
        "Upgrade": "websocket",
        "Connection": "Upgrade"
      });
    };
  }

  globalThis.__slice_dispatch_server_ws = function(eventType, isBinary, payloadStrOrB64, code, reason) {
    var h = globalThis.__slice_ws_server_handler
      || (globalThis.__slice_default_export && globalThis.__slice_default_export.websocket)
      || null;
    var wsPeer = {
      readyState: 1,
      send: function(data) {
        if (typeof data === "string") {
          __slice_ws_server_send(false, data);
        } else {
          var u8 = __slice_toU8(data);
          __slice_ws_server_send(true, __slice_base64_encode_bytes(u8, false));
        }
      },
      close: function(c, r) {
        wsPeer.readyState = 3;
        __slice_ws_server_close(c !== undefined ? Number(c) | 0 : 1000, r !== undefined ? String(r) : "");
      }
    };
    if (!h) {
      if (eventType === "message") {
        if (isBinary) {
          __slice_ws_server_send(true, payloadStrOrB64);
        } else {
          __slice_ws_server_send(false, payloadStrOrB64);
        }
      }
      return;
    }
    if (typeof h === "function") {
      if (eventType === "message") {
        var arg = isBinary ? __slice_base64_decode_bytes(payloadStrOrB64) : payloadStrOrB64;
        var ret = h(arg, wsPeer);
        if (ret !== undefined && ret !== null) wsPeer.send(ret);
      }
      return;
    }
    if (eventType === "open" && typeof h.open === "function") {
      h.open(wsPeer);
    } else if (eventType === "message" && typeof h.message === "function") {
      var msgArg = isBinary ? __slice_base64_decode_bytes(payloadStrOrB64) : payloadStrOrB64;
      var out = h.message(wsPeer, msgArg);
      if (out !== undefined && out !== null && typeof out.then !== "function") {
        wsPeer.send(out);
      }
    } else if (eventType === "close" && typeof h.close === "function") {
      h.close(wsPeer, code || 1000, reason || "");
    }
  };
})();

