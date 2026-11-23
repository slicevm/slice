// Slice (Nexus-JS) Lazy Prelude Shard 1: WinterTC `URL` and `URLSearchParams` (`01_web_fetch_url.js`)
(function() {
  function __uspD(s) {
    if (s.indexOf("+") === -1 && s.indexOf("%") === -1) return s;
    return __slice_usp_decode(s);
  }

  var __uspEncCache = Object.create(null);
  var __uspEncCount = 0;

  function __uspE(s) {
    var str = typeof s === "string" ? s : String(s), n = str.length;
    if (n === 0) return "";
    if (n <= 48) {
      var hit = __uspEncCache[str];
      if (hit !== undefined) return hit;
    }
    var out;
    if (n <= 64) {
      var safe = true;
      for (var i = 0; i < n; i++) {
        var c = str.charCodeAt(i);
        if (
          !(
            (c >= 97 && c <= 122) ||
            (c >= 65 && c <= 90) ||
            (c >= 48 && c <= 57) ||
            c === 45 ||
            c === 95 ||
            c === 46 ||
            c === 126
          )
        ) {
          safe = false;
          break;
        }
      }
      out = safe ? str : __slice_usp_encode(str);
    } else {
      out = __slice_usp_encode(str);
    }
    if (n <= 48 && __uspEncCount < 512) {
      __uspEncCache[str] = out;
      __uspEncCount++;
    }
    return out;
  }

  function URLSearchParams(init, u) {
    this._u = u || null;
    this._raw = "";
    if (!init) {
      this._p = [];
      return;
    }
    if (typeof init === "string") {
      var q = init.charCodeAt(0) === 63 ? init.slice(1) : init;
      if (q.length === 0) {
        this._p = [];
      } else {
        this._raw = q;
        this._p = null;
      }
      return;
    }
    this._p = [];
    if (Array.isArray(init)) {
      for (var j = 0; j < init.length; j++) {
        var pr = init[j];
        if (!pr || typeof pr !== "object" || pr.length !== 2) {
          throw new TypeError("URLSearchParams: each sequence item must be a 2-element pair");
        }
        this._p.push([String(pr[0]), String(pr[1])]);
      }
    } else if (typeof init === "object") {
      if (init._isUSP) {
        var sp = init._ensureP();
        for (var m = 0; m < sp.length; m++) this._p.push([sp[m][0], sp[m][1]]);
      } else if (typeof Symbol !== "undefined" && Symbol.iterator && typeof init[Symbol.iterator] === "function") {
        var iter = init[Symbol.iterator](), step;
        while (!(step = iter.next()).done) {
          var item = step.value;
          if (!item || typeof item !== "object") {
            throw new TypeError("URLSearchParams: sequence item must be an object pair");
          }
          var pairArr = Array.isArray(item) ? item : Array.from(item);
          if (pairArr.length !== 2) {
            throw new TypeError("URLSearchParams: sequence item must contain exactly 2 elements");
          }
          this._p.push([String(pairArr[0]), String(pairArr[1])]);
        }
      } else {
        var ks = Object.keys(init);
        for (var k = 0; k < ks.length; k++) this._p.push([ks[k], String(init[ks[k]])]);
      }
    }
  }
  URLSearchParams.prototype._isUSP = true;
  URLSearchParams.prototype._ensureP = function() {
    var p = this._p;
    if (p !== null) return p;
    p = [];
    this._p = p;
    var q = this._raw;
    this._raw = "";
    if (q.length > 0) {
      var ps = q.split("&");
      for (var i = 0; i < ps.length; i++) {
        var sg = ps[i];
        if (!sg) continue;
        var eq = sg.indexOf("=");
        if (eq === -1) p.push([__uspD(sg), ""]);
        else p.push([__uspD(sg.slice(0, eq)), __uspD(sg.slice(eq + 1))]);
      }
    }
    return p;
  };
  URLSearchParams.prototype._sync = function() {
    var u = this._u;
    if (u) {
      if (u._isFastUrl) u._sd = true;
      else u._syncSearch();
    }
  };
  URLSearchParams.prototype.get = function(n) {
    var k = typeof n === "string" ? n : String(n);
    var p = this._p;
    if (p === null) {
      var q = this._raw;
      if (!q) return null;
      if (q.indexOf("+") === -1 && q.indexOf("%") === -1) {
        var qLen = q.length, kLen = k.length, pos = 0;
        while (pos < qLen) {
          var amp = q.indexOf("&", pos);
          var end = amp === -1 ? qLen : amp;
          var sgLen = end - pos;
          if (sgLen > 0) {
            if (sgLen > kLen && q.charCodeAt(pos + kLen) === 61 && q.startsWith(k, pos)) {
              return q.slice(pos + kLen + 1, end);
            }
            if (sgLen === kLen && q.startsWith(k, pos)) {
              return "";
            }
          }
          if (amp === -1) break;
          pos = amp + 1;
        }
        return null;
      }
      p = this._ensureP();
    }
    for (var i = 0; i < p.length; i++) {
      if (p[i][0] === k) return p[i][1];
    }
    return null;
  };
  URLSearchParams.prototype.getAll = function(n) {
    var k = typeof n === "string" ? n : String(n);
    var p = this._p;
    if (p === null) {
      var q = this._raw;
      if (!q) return [];
      if (q.indexOf("+") === -1 && q.indexOf("%") === -1) {
        var rFast = [], qLen = q.length, kLen = k.length, pos = 0;
        while (pos < qLen) {
          var amp = q.indexOf("&", pos);
          var end = amp === -1 ? qLen : amp;
          var sgLen = end - pos;
          if (sgLen > 0) {
            if (sgLen > kLen && q.charCodeAt(pos + kLen) === 61 && q.startsWith(k, pos)) {
              rFast.push(q.slice(pos + kLen + 1, end));
            } else if (sgLen === kLen && q.startsWith(k, pos)) {
              rFast.push("");
            }
          }
          if (amp === -1) break;
          pos = amp + 1;
        }
        return rFast;
      }
      p = this._ensureP();
    }
    var r = [];
    for (var i = 0; i < p.length; i++) {
      if (p[i][0] === k) r.push(p[i][1]);
    }
    return r;
  };
  URLSearchParams.prototype.has = function(n) {
    if (this._p === null) return this.get(n) !== null;
    var k = typeof n === "string" ? n : String(n), p = this._p;
    for (var i = 0; i < p.length; i++) {
      if (p[i][0] === k) return true;
    }
    return false;
  };
  URLSearchParams.prototype.set = function(n, v) {
    var k = typeof n === "string" ? n : String(n);
    var s = typeof v === "string" ? v : String(v);
    var p = this._ensureP(), len = p.length, fi = -1;
    for (var i = 0; i < len; i++) {
      if (p[i][0] === k) {
        if (fi === -1) {
          p[i][1] = s;
          fi = i;
        } else {
          p.splice(i, 1);
          i--;
          len--;
        }
      }
    }
    if (fi === -1) p.push([k, s]);
    var u = this._u;
    if (u) {
      if (u._isFastUrl) u._sd = true;
      else u._syncSearch();
    }
  };
  URLSearchParams.prototype.append = function(n, v) {
    this._ensureP().push([typeof n === "string" ? n : String(n), typeof v === "string" ? v : String(v)]);
    var u = this._u;
    if (u) {
      if (u._isFastUrl) u._sd = true;
      else u._syncSearch();
    }
  };
  URLSearchParams.prototype.delete = function(n) {
    var k = typeof n === "string" ? n : String(n), p = this._ensureP(), nx = [];
    for (var i = 0; i < p.length; i++) {
      if (p[i][0] !== k) nx.push(p[i]);
    }
    this._p = nx;
    var u = this._u;
    if (u) {
      if (u._isFastUrl) u._sd = true;
      else u._syncSearch();
    }
  };
  URLSearchParams.prototype.sort = function() {
    var p = this._ensureP(), n = p.length;
    if (n > 1) {
      if (n <= 12) {
        for (var i = 1; i < n; i++) {
          var cur = p[i], ck = cur[0], j = i - 1;
          while (j >= 0 && p[j][0] > ck) {
            p[j + 1] = p[j];
            j--;
          }
          p[j + 1] = cur;
        }
      } else {
        p.sort(function(a, b) {
          return a[0] < b[0] ? -1 : (a[0] > b[0] ? 1 : 0);
        });
      }
    }
    var u = this._u;
    if (u) {
      if (u._isFastUrl) u._sd = true;
      else u._syncSearch();
    }
  };
  URLSearchParams.prototype.forEach = function(cb, ta) {
    var p = this._ensureP();
    for (var i = 0; i < p.length; i++) {
      cb.call(ta, p[i][1], p[i][0], this);
    }
  };
  var __uspIterProto = Object.create(Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]())));
  __uspIterProto.next = function() {
    var p = this._t._ensureP();
    if (this._i >= p.length) return { value: undefined, done: true };
    var pair = p[this._i++];
    var k = this._k;
    return {
      value: k === 0 ? pair[0] : (k === 1 ? pair[1] : [pair[0], pair[1]]),
      done: false
    };
  };
  if (typeof Symbol !== "undefined" && Symbol.toStringTag) {
    Object.defineProperty(__uspIterProto, Symbol.toStringTag, {
      value: "URLSearchParams Iterator",
      configurable: true
    });
  }
  function __mkUspIter(usp, kind) {
    var it = Object.create(__uspIterProto);
    it._t = usp;
    it._i = 0;
    it._k = kind;
    return it;
  }

  URLSearchParams.prototype.entries = function() {
    return __mkUspIter(this, 2);
  };
  URLSearchParams.prototype.keys = function() {
    return __mkUspIter(this, 0);
  };
  URLSearchParams.prototype.values = function() {
    return __mkUspIter(this, 1);
  };
  URLSearchParams.prototype.toString = function() {
    var p = this._ensureP(), len = p.length;
    if (len === 0) return "";
    var s = "";
    for (var i = 0; i < len; i++) {
      if (i > 0) s += "&";
      s += __uspE(p[i][0]) + "=" + __uspE(p[i][1]);
    }
    return s;
  };
  if (typeof Symbol !== "undefined" && Symbol.iterator) {
    URLSearchParams.prototype[Symbol.iterator] = URLSearchParams.prototype.entries;
  }
  Object.defineProperty(URLSearchParams.prototype, "size", {
    get: function() { return this._ensureP().length; }
  });
