// Slice (Nexus-JS) Lazy Prelude Shard 12: WHATWG `URLPattern` (`12_url_pattern.js`)
(function() {
  if (!globalThis.URL) __slice_ensure_shard(1);
  var URL = globalThis.URL;

  function __escRe(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  }

  function __compileUrlPat(pat, sep, ignoreCase) {
    var p = String(pat === undefined ? "*" : pat);
    if (p === "*") {
      return { re: new RegExp("^(.*)$", ignoreCase ? "i" : ""), keys: ["0"], hasRx: false };
    }
    var keys = [], hasRx = false, rx = "^", i = 0, n = p.length, anonIdx = 0;
    var segCh = sep ? "[^" + __escRe(sep) + "]" : ".";
    while (i < n) {
      var ch = p.charAt(i);
      if (ch === "\\") {
        if (i + 1 < n) {
          rx += __escRe(p.charAt(i + 1));
          i += 2;
        } else {
          rx += "\\\\";
          i++;
        }
        continue;
      }
      if (ch === "*") {
        var prevSlash = sep && i > 0 && p.charAt(i - 1) === sep && rx.slice(-__escRe(sep).length) === __escRe(sep);
        keys.push(String(anonIdx++));
        if (prevSlash && i + 1 === n) {
          rx = rx.slice(0, -__escRe(sep).length) + "(?:" + __escRe(sep) + "(.*))?";
        } else {
          rx += "(.*)";
        }
        i++;
        continue;
      }
      if (ch === ":") {
        var j = i + 1;
        while (j < n && /[a-zA-Z0-9_]/.test(p.charAt(j))) j++;
        var name = p.slice(i + 1, j);
        if (!name) throw new TypeError("Invalid URLPattern parameter at index " + i);
        keys.push(name);
        var customRe = null;
        if (j < n && p.charAt(j) === "(") {
          var depth = 1, k = j + 1;
          while (k < n && depth > 0) {
            if (p.charAt(k) === "\\") { k += 2; continue; }
            if (p.charAt(k) === "(") depth++;
            else if (p.charAt(k) === ")") depth--;
            k++;
          }
          if (depth !== 0) throw new TypeError("Unclosed regex group in URLPattern");
          customRe = p.slice(j + 1, k - 1);
          hasRx = true;
          j = k;
        }
        var mod = j < n && (p.charAt(j) === "?" || p.charAt(j) === "+" || p.charAt(j) === "*") ? p.charAt(j++) : "";
        var baseUnit = customRe ? "(?:" + customRe + ")" : segCh + "+";
        var hasLeadingSep = sep && rx.length > 1 && rx.slice(-__escRe(sep).length) === __escRe(sep);
        if (mod === "?") {
          if (hasLeadingSep) {
            rx = rx.slice(0, -__escRe(sep).length) + "(?:" + __escRe(sep) + "(" + baseUnit + "))?";
          } else {
            rx += "(" + baseUnit + ")?";
          }
        } else if (mod === "+") {
          var sepEsc = sep ? __escRe(sep) : "";
          rx += "(" + baseUnit + "(?:" + sepEsc + baseUnit + ")*)";
        } else if (mod === "*") {
          var sepEsc2 = sep ? __escRe(sep) : "";
          if (hasLeadingSep) {
            rx = rx.slice(0, -__escRe(sep).length) + "(?:" + sepEsc2 + "(" + baseUnit + "(?:" + sepEsc2 + baseUnit + ")*))?";
          } else {
            rx += "(" + (customRe ? "(?:" + customRe + ")*" : ".*") + ")";
          }
        } else {
          rx += "(" + baseUnit + ")";
        }
        i = j;
        continue;
      }
      if (ch === "(") {
        var d2 = 1, k2 = i + 1;
        while (k2 < n && d2 > 0) {
          if (p.charAt(k2) === "\\") { k2 += 2; continue; }
          if (p.charAt(k2) === "(") d2++;
          else if (p.charAt(k2) === ")") d2--;
          k2++;
        }
        if (d2 !== 0) throw new TypeError("Unclosed group in URLPattern");
        var inner = p.slice(i + 1, k2 - 1);
        var mod2 = k2 < n && (p.charAt(k2) === "?" || p.charAt(k2) === "+" || p.charAt(k2) === "*") ? p.charAt(k2++) : "";
        keys.push(String(anonIdx++));
        hasRx = true;
        rx += "((?:" + inner + "))" + mod2;
        i = k2;
        continue;
      }
      if (ch === "{") {
        var closeIdx = p.indexOf("}", i + 1);
        if (closeIdx === -1) throw new TypeError("Unclosed brace in URLPattern");
        var braceInner = p.slice(i + 1, closeIdx);
        var mod3 = closeIdx + 1 < n && (p.charAt(closeIdx + 1) === "?" || p.charAt(closeIdx + 1) === "+" || p.charAt(closeIdx + 1) === "*") ? p.charAt(closeIdx + 1) : "";
        var sub = __compileUrlPat(braceInner, sep, ignoreCase);
        for (var sk = 0; sk < sub.keys.length; sk++) {
          keys.push(sub.keys[sk] === "0" ? String(anonIdx++) : sub.keys[sk]);
        }
        if (sub.hasRx) hasRx = true;
        var subSrc = sub.re.source.slice(1, -1);
        rx += "(?:" + subSrc + ")" + mod3;
        i = closeIdx + 1 + (mod3 ? 1 : 0);
        continue;
      }
      rx += __escRe(ch);
      i++;
    }
    rx += "$";
    return { re: new RegExp(rx, ignoreCase ? "i" : ""), keys: keys, hasRx: hasRx };
  }

  function __findUrlPatQueryIdx(s) {
    var paren = 0, brace = 0, inParam = false;
    for (var i = 0; i < s.length; i++) {
      var c = s.charAt(i);
      if (c === "\\") { i++; continue; }
      if (c === "(") { paren++; inParam = false; continue; }
      if (c === ")") {
        if (paren > 0) paren--;
        if (paren === 0 && i + 1 < s.length && s.charAt(i + 1) === "?") { i++; continue; }
        continue;
      }
      if (paren > 0) continue;
      if (c === "{") { brace++; inParam = false; continue; }
      if (c === "}") {
        if (brace > 0) brace--;
        if (brace === 0 && i + 1 < s.length && s.charAt(i + 1) === "?") { i++; continue; }
        continue;
      }
      if (brace > 0) continue;
      if (c === ":" && !(i + 2 < s.length && s.slice(i, i + 3) === "://")) {
        inParam = true;
        continue;
      }
      if (inParam) {
        if (/[a-zA-Z0-9_]/.test(c)) continue;
        inParam = false;
        if (c === "?") continue;
      }
      if (c === "?") return i;
    }
    return -1;
  }

  function __parseUrlPatternStr(str, baseURL) {
    var s = String(str).trim();
    var init = {
      protocol: "*",
      username: "*",
      password: "*",
      hostname: "*",
      port: "*",
      pathname: "*",
      search: "*",
      hash: "*"
    };
    if (baseURL !== undefined) {
      var b = new URL(String(baseURL));
      init.protocol = b.protocol.replace(/:$/, "");
      init.username = b.username || "*";
      init.password = b.password || "*";
      init.hostname = b.hostname;
      init.port = b.port || "";
      init.pathname = b.pathname;
    }
    var hi = s.indexOf("#");
    if (hi !== -1) {
      init.hash = s.slice(hi + 1);
      s = s.slice(0, hi);
    }
    var qi = __findUrlPatQueryIdx(s);
    if (qi !== -1) {
      init.search = s.slice(qi + 1);
      s = s.slice(0, qi);
    }
    var si = s.indexOf("://");
    if (si !== -1) {
      init.protocol = s.slice(0, si);
      var rest = s.slice(si + 3);
      var sl = rest.indexOf("/");
      var auth = sl === -1 ? rest : rest.slice(0, sl);
      init.pathname = sl === -1 ? "/" : rest.slice(sl);
      var at = auth.indexOf("@");
      if (at !== -1) {
        var ui = auth.slice(0, at);
        auth = auth.slice(at + 1);
        var cl = ui.indexOf(":");
        if (cl === -1) {
          init.username = ui;
        } else {
          init.username = ui.slice(0, cl);
          init.password = ui.slice(cl + 1);
        }
      }
      var pCol = auth.lastIndexOf(":");
      if (pCol !== -1 && auth.indexOf("]") < pCol) {
        init.hostname = auth.slice(0, pCol);
        init.port = auth.slice(pCol + 1);
      } else {
        init.hostname = auth;
        init.port = "*";
      }
    } else if (s.length > 0) {
      if (s.charCodeAt(0) === 47) {
        init.pathname = s;
      } else if (baseURL !== undefined) {
        var dir = init.pathname.slice(0, init.pathname.lastIndexOf("/") + 1) || "/";
        init.pathname = dir + s;
      } else {
        init.pathname = "/" + s;
      }
    }
    return init;
  }

  function URLPattern(input, baseURL, options) {
    if (!(this instanceof URLPattern)) {
      throw new TypeError("Failed to construct 'URLPattern': Please use the 'new' operator");
    }
    var opts = options;
    var base = baseURL;
    if (base && typeof base === "object" && options === undefined) {
      opts = base;
      base = undefined;
    }
    var ignoreCase = Boolean(opts && opts.ignoreCase);
    var init;
    if (input === undefined) {
      init = { protocol: "*", username: "*", password: "*", hostname: "*", port: "*", pathname: "*", search: "*", hash: "*" };
    } else if (typeof input === "string" || input instanceof URL) {
      init = __parseUrlPatternStr(input instanceof URL ? input.href : input, base);
    } else if (typeof input === "object" && input !== null) {
      if (base !== undefined) {
        throw new TypeError("Failed to construct 'URLPattern': Second argument cannot be a baseURL when first argument is a dictionary");
      }
      var bObj = input.baseURL ? new URL(String(input.baseURL)) : null;
      init = {
        protocol: input.protocol !== undefined ? String(input.protocol).replace(/:$/, "") : (bObj ? bObj.protocol.replace(/:$/, "") : "*"),
        username: input.username !== undefined ? String(input.username) : (bObj && bObj.username ? bObj.username : "*"),
        password: input.password !== undefined ? String(input.password) : (bObj && bObj.password ? bObj.password : "*"),
        hostname: input.hostname !== undefined ? String(input.hostname) : (bObj ? bObj.hostname : "*"),
        port: input.port !== undefined ? String(input.port) : (bObj ? bObj.port : "*"),
        pathname: input.pathname !== undefined ? String(input.pathname) : (bObj ? bObj.pathname : "*"),
        search: input.search !== undefined ? String(input.search).replace(/^\?/, "") : "*",
        hash: input.hash !== undefined ? String(input.hash).replace(/^#/, "") : "*"
      };
    } else {
      throw new TypeError("Failed to construct 'URLPattern': Invalid input");
    }
    this.protocol = init.protocol;
    this.username = init.username;
    this.password = init.password;
    this.hostname = init.hostname;
    this.port = init.port;
    this.pathname = init.pathname;
    this.search = init.search;
    this.hash = init.hash;
    this._c = {
      protocol: __compileUrlPat(this.protocol, "", true),
      username: __compileUrlPat(this.username, "", ignoreCase),
      password: __compileUrlPat(this.password, "", ignoreCase),
      hostname: __compileUrlPat(this.hostname, ".", true),
      port: __compileUrlPat(this.port, "", false),
      pathname: __compileUrlPat(this.pathname, "/", ignoreCase),
      search: __compileUrlPat(this.search, "", ignoreCase),
      hash: __compileUrlPat(this.hash, "", ignoreCase)
    };
    this.hasRegExpGroups = Boolean(
      this._c.protocol.hasRx || this._c.username.hasRx || this._c.password.hasRx ||
      this._c.hostname.hasRx || this._c.port.hasRx || this._c.pathname.hasRx ||
      this._c.search.hasRx || this._c.hash.hasRx
    );
  }

  URLPattern.prototype._normTarget = function(input, baseURL) {
    if (typeof input === "string" || input instanceof URL) {
      var raw = input instanceof URL ? input.href : String(input);
      try {
        var u = baseURL !== undefined
          ? new URL(raw, baseURL)
          : (raw.indexOf("://") === -1 && raw.charCodeAt(0) === 47 && this.protocol === "*" && this.hostname === "*"
              ? new URL(raw, "http://localhost")
              : new URL(raw));
        return {
          inputs: baseURL !== undefined ? [String(input), String(baseURL)] : [String(input)],
          protocol: u.protocol.replace(/:$/, ""),
          username: u.username || "",
          password: u.password || "",
          hostname: u.hostname || "",
          port: u.port || "",
          pathname: u.pathname || "/",
          search: u.search ? u.search.slice(1) : "",
          hash: u.hash ? u.hash.slice(1) : ""
        };
      } catch (_e) {
        return null;
      }
    }
    if (input && typeof input === "object") {
      if (baseURL !== undefined) {
        throw new TypeError("URLPattern: baseURL is not allowed when input is an object");
      }
      if (typeof input.url === "string" && input.pathname === undefined) {
        return this._normTarget(input.url);
      }
      var b = input.baseURL ? new URL(String(input.baseURL)) : null;
      return {
        inputs: [input],
        protocol: input.protocol !== undefined ? String(input.protocol).replace(/:$/, "") : (b ? b.protocol.replace(/:$/, "") : ""),
        username: input.username !== undefined ? String(input.username) : (b ? b.username : ""),
        password: input.password !== undefined ? String(input.password) : (b ? b.password : ""),
        hostname: input.hostname !== undefined ? String(input.hostname) : (b ? b.hostname : ""),
        port: input.port !== undefined ? String(input.port) : (b ? b.port : ""),
        pathname: input.pathname !== undefined ? String(input.pathname) : (b ? b.pathname : "/"),
        search: input.search !== undefined ? String(input.search).replace(/^\?/, "") : (b && b.search ? b.search.slice(1) : ""),
        hash: input.hash !== undefined ? String(input.hash).replace(/^#/, "") : (b && b.hash ? b.hash.slice(1) : "")
      };
    }
    return null;
  };

  URLPattern.prototype.exec = function(input, baseURL) {
    var t = this._normTarget(input, baseURL);
    if (!t) return null;
    var parts = ["protocol", "username", "password", "hostname", "port", "pathname", "search", "hash"];
    var res = { inputs: t.inputs };
    for (var i = 0; i < 8; i++) {
      var k = parts[i];
      var comp = this._c[k];
      var val = t[k];
      var m = comp.re.exec(val);
      if (!m) return null;
      var groups = {};
      for (var j = 0; j < comp.keys.length; j++) {
        groups[comp.keys[j]] = m[j + 1];
      }
      res[k] = { input: val, groups: groups };
    }
    return res;
  };

  URLPattern.prototype.test = function(input, baseURL) {
    return this.exec(input, baseURL) !== null;
  };

  globalThis.URLPattern = URLPattern;
})();

