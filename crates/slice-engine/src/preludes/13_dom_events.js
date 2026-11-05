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
    this._et = Object.create(null);
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

  globalThis.Event = Event;
  globalThis.CustomEvent = CustomEvent;
  globalThis.EventTarget = EventTarget;
})();

