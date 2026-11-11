// Slice (Nexus-JS) Lazy Prelude Shard 13: DOM Events (`Event`, `CustomEvent`, `EventTarget`) (`13_dom_events.js`)
(function() {
  function Event(type, init) {
    if (arguments.length === 0) throw new TypeError("Event constructor requires a type argument");
    var o = init || {};
    this.type = String(type);
    this.bubbles = Boolean(o.bubbles);
    this.cancelable = Boolean(o.cancelable);
    this.composed = Boolean(o.composed);
    this.defaultPrevented = false;
    this.target = null;
    this.currentTarget = null;
    this.srcElement = null;
    this.eventPhase = 0;
    this.isTrusted = false;
    this.timeStamp = performance.now();
    this._stop = false;
    this._stopImm = false;
  }
  Event.prototype.preventDefault = function() {
    if (this.cancelable) this.defaultPrevented = true;
  };
  Event.prototype.stopPropagation = function() {
    this._stop = true;
  };
  Event.prototype.stopImmediatePropagation = function() {
    this._stop = true;
    this._stopImm = true;
  };
  Event.prototype.composedPath = function() {
    return this.currentTarget ? [this.currentTarget] : [];
  };

  function CustomEvent(type, init) {
    Event.call(this, type, init);
    this.detail = (init && init.detail !== undefined) ? init.detail : null;
  }
  CustomEvent.prototype = Object.create(Event.prototype);
  CustomEvent.prototype.constructor = CustomEvent;

  function EventTarget() {
    this._et = null;
  }
  EventTarget.prototype._ensureEt = function() {
    if (!this._et) this._et = Object.create(null);
    return this._et;
  };
  EventTarget.prototype.addEventListener = function(type, listener, options) {
    if (listener === null || listener === undefined) return;
    if (typeof listener !== "function" && (typeof listener !== "object" || typeof listener.handleEvent !== "function")) {
      return;
    }
    var o = (typeof options === "object" && options !== null) ? options : { capture: Boolean(options) };
    var sig = o.signal || null;
    if (sig && sig.aborted) return;
    var t = String(type);
    var map = this._ensureEt();
    var list = map[t] || (map[t] = []);
    for (var i = 0; i < list.length; i++) {
      if (list[i].fn === listener) return;
    }
    list.push({ fn: listener, once: Boolean(o.once) });
    if (sig && typeof sig.addEventListener === "function") {
      var self = this;
      sig.addEventListener("abort", function() {
        self.removeEventListener(t, listener);
      });
    }
  };
  EventTarget.prototype.removeEventListener = function(type, listener) {
    if (!this._et) return;
    var t = String(type), list = this._et[t];
    if (!list) return;
    var nx = [];
    for (var i = 0; i < list.length; i++) {
      if (list[i].fn !== listener) nx.push(list[i]);
    }
    this._et[t] = nx;
  };
  EventTarget.prototype.dispatchEvent = function(event) {
    if (!event || typeof event.type !== "string") throw new TypeError("dispatchEvent requires an Event");
    event.target = this;
    event.currentTarget = this;
    event.srcElement = this;
    var t = event.type;
    var onProp = this["on" + t];
    if (typeof onProp === "function") {
      try { onProp.call(this, event); } catch (_e) {}
    }
    if (this._et && this._et[t]) {
      var snapshot = this._et[t].slice();
      for (var i = 0; i < snapshot.length; i++) {
        if (event._stopImm) break;
        var item = snapshot[i];
        if (item.once) this.removeEventListener(t, item.fn);
        try {
          if (typeof item.fn === "function") item.fn.call(this, event);
          else if (item.fn && typeof item.fn.handleEvent === "function") item.fn.handleEvent(event);
        } catch (_e2) {}
      }
    }
    return !event.defaultPrevented;
  };

  var __domCodes = {
    IndexSizeError: 1,
    HierarchyRequestError: 3,
    WrongDocumentError: 4,
    InvalidCharacterError: 5,
    NoModificationAllowedError: 7,
    NotFoundError: 8,
    NotSupportedError: 9,
    InUseAttributeError: 10,
    InvalidStateError: 11,
    SyntaxError: 12,
    InvalidModificationError: 13,
    NamespaceError: 14,
    InvalidAccessError: 15,
    TypeMismatchError: 17,
    SecurityError: 18,
    NetworkError: 19,
    AbortError: 20,
    URLMismatchError: 21,
    QuotaExceededError: 22,
    TimeoutError: 23,
    InvalidNodeTypeError: 24,
    DataCloneError: 25
  };
  var __domConsts = {
    INDEX_SIZE_ERR: 1,
    DOMSTRING_SIZE_ERR: 2,
    HIERARCHY_REQUEST_ERR: 3,
    WRONG_DOCUMENT_ERR: 4,
    INVALID_CHARACTER_ERR: 5,
    NO_DATA_ALLOWED_ERR: 6,
    NO_MODIFICATION_ALLOWED_ERR: 7,
    NOT_FOUND_ERR: 8,
    NOT_SUPPORTED_ERR: 9,
    INUSE_ATTRIBUTE_ERR: 10,
    INVALID_STATE_ERR: 11,
    SYNTAX_ERR: 12,
    INVALID_MODIFICATION_ERR: 13,
    NAMESPACE_ERR: 14,
    INVALID_ACCESS_ERR: 15,
    VALIDATION_ERR: 16,
    TYPE_MISMATCH_ERR: 17,
    SECURITY_ERR: 18,
    NETWORK_ERR: 19,
    ABORT_ERR: 20,
    URL_MISMATCH_ERR: 21,
    QUOTA_EXCEEDED_ERR: 22,
    TIMEOUT_ERR: 23,
    INVALID_NODE_TYPE_ERR: 24,
    DATA_CLONE_ERR: 25
  };
  function DOMException(message, name) {
    if (!(this instanceof DOMException)) return new DOMException(message, name);
    this.message = message === undefined ? "" : String(message);
    this.name = name === undefined ? "Error" : String(name);
    this.code = __domCodes[this.name] || 0;
  }
  DOMException.prototype = Object.create(Error.prototype);
  DOMException.prototype.constructor = DOMException;
  Object.defineProperty(DOMException.prototype, "stack", {
    get: function() {
      var nm = this && this.name ? String(this.name) : "Error";
      var msg = this && this.message ? String(this.message) : "";
      return msg ? (nm + ": " + msg) : nm;
    },
    set: function(v) {
      Object.defineProperty(this, "stack", { value: v, writable: true, configurable: true });
    },
    configurable: true,
    enumerable: false
  });
  Object.assign(DOMException, __domConsts);
  Object.assign(DOMException.prototype, __domConsts);

  globalThis.DOMException = DOMException;
  globalThis.Event = Event;
  globalThis.CustomEvent = CustomEvent;
  globalThis.EventTarget = EventTarget;
})();

