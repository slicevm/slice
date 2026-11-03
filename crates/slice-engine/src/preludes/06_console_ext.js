// Slice (Nexus-JS) Lazy Prelude Shard 6: Extended `console.*` (`table`, `dir`, `group`, `time`, `count`, `trace`, `assert`) (`06_console_ext.js`)
(function() {
  var __U8 = globalThis.Uint8Array;
  var __cDepth = 0;
  var __cTimers = Object.create(null);
  var __cCounts = Object.create(null);

  function __cInsp(val, depth, maxDepth, seen) {
    if (val === null) return "null";
    if (val === undefined) return "undefined";
    var t = typeof val;
    if (t === "string") return depth === 0 ? val : "'" + val + "'";
    if (t !== "object") return String(val);
    if (val instanceof Error) return val.name + ": " + val.message;
    if (val instanceof Date) return val.toISOString();
    if (val instanceof RegExp) return val.toString();
    if (val._isBuffer === true) return "<Buffer " + val.toString("hex") + ">";
    if (val instanceof __U8) return "Uint8Array(" + val.length + ") [" + Array.from(val).join(", ") + "]";
    seen = seen || [];
    if (seen.indexOf(val) !== -1) return "[Circular]";
    if (maxDepth !== null && maxDepth !== undefined && depth > maxDepth) {
      return Array.isArray(val) ? "[Array]" : "[Object]";
    }
    seen.push(val);
    if (Array.isArray(val)) {
      var a = [];
      for (var i = 0; i < val.length; i++) a.push(__cInsp(val[i], depth + 1, maxDepth, seen));
      seen.pop();
      return "[" + a.join(", ") + "]";
    }
    var ks = Object.keys(val), p = [];
    for (var j = 0; j < ks.length; j++) {
      var k = ks[j];
      p.push(k + ": " + __cInsp(val[k], depth + 1, maxDepth, seen));
    }
    seen.pop();
    return p.length === 0 ? "{}" : "{ " + p.join(", ") + " }";
  }

  function __cPad(str, width) {
    var s = String(str);
    while (s.length < width) s += " ";
    return s;
  }

  function __cEmitArgs(args) {
    if (__cDepth === 0) {
      if (args.length === 1 && (typeof args[0] !== "object" || args[0] === null)) {
        __slice_console_log(args[0]);
        return;
      }
      var parts0 = [];
      for (var i0 = 0; i0 < args.length; i0++) {
        var a0 = args[i0];
        parts0.push((typeof a0 === "object" && a0 !== null) ? __cInsp(a0, 0, 4, []) : String(a0));
      }
      __slice_console_log(parts0.join(" "));
      return;
    }
    var indent = "";
    for (var d = 0; d < __cDepth; d++) indent += "  ";
    var parts = [];
    for (var i = 0; i < args.length; i++) {
      var a = args[i];
      parts.push((typeof a === "object" && a !== null) ? __cInsp(a, 0, 4, []) : String(a));
    }
    __slice_console_log(indent + parts.join(" "));
  }

  function __cLog() {
    __cEmitArgs(arguments);
  }

  globalThis.console = {
    log: __cLog,
    info: __cLog,
    warn: __cLog,
    error: __cLog,
    debug: __cLog,
    dirxml: __cLog,
    dir: function(obj, options) {
      var maxD = (options && options.depth !== undefined) ? options.depth : 2;
      __cEmitArgs([__cInsp(obj, 1, maxD, [])]);
    },
    trace: function() {
      var parts = ["Trace:"];
      for (var i = 0; i < arguments.length; i++) parts.push(__cInsp(arguments[i], 0, 2, []));
      __cEmitArgs([parts.join(" ")]);
    },
    assert: function(condition) {
      if (condition) return;
      var rest = [];
      for (var i = 1; i < arguments.length; i++) rest.push(__cInsp(arguments[i], 0, 2, []));
      var msg = rest.length > 0 ? "Assertion failed: " + rest.join(" ") : "Assertion failed";
      __cEmitArgs([msg]);
    },
    clear: function() {
      __cDepth = 0;
    },
    count: function(label) {
      var k = label === undefined ? "default" : String(label);
      var n = (__cCounts[k] || 0) + 1;
      __cCounts[k] = n;
      __cEmitArgs([k + ": " + n]);
    },
    countReset: function(label) {
      var k = label === undefined ? "default" : String(label);
      delete __cCounts[k];
    },
    group: function() {
      if (arguments.length > 0) __cEmitArgs(arguments);
      __cDepth++;
    },
    groupCollapsed: function() {
      if (arguments.length > 0) __cEmitArgs(arguments);
      __cDepth++;
    },
    groupEnd: function() {
      if (__cDepth > 0) __cDepth--;
    },
    time: function(label) {
      var k = label === undefined ? "default" : String(label);
      __cTimers[k] = performance.now();
    },
    timeLog: function(label) {
      var k = label === undefined ? "default" : String(label);
      var t0 = __cTimers[k];
      if (t0 === undefined) {
        __cEmitArgs(["Warning: No such label '" + k + "' for console.timeLog()"]);
        return;
      }
      var elapsed = Math.max(0, performance.now() - t0);
      var rest = [k + ": " + elapsed.toFixed(3) + "ms"];
      for (var i = 1; i < arguments.length; i++) rest.push(__cInsp(arguments[i], 0, 2, []));
      __cEmitArgs([rest.join(" ")]);
    },
    timeEnd: function(label) {
      var k = label === undefined ? "default" : String(label);
      var t0 = __cTimers[k];
      if (t0 === undefined) {
        __cEmitArgs(["Warning: No such label '" + k + "' for console.timeEnd()"]);
        return;
      }
      var elapsed = Math.max(0, performance.now() - t0);
      delete __cTimers[k];
      __cEmitArgs([k + ": " + elapsed.toFixed(3) + "ms"]);
    },
    table: function(tabularData, properties) {
      if (tabularData === null || typeof tabularData !== "object") {
        __cEmitArgs([tabularData]);
        return;
      }
      var isArr = Array.isArray(tabularData);
      var rowKeys = isArr ? [] : Object.keys(tabularData);
      if (isArr) {
        for (var r = 0; r < tabularData.length; r++) rowKeys.push(String(r));
      }
      if (rowKeys.length === 0) {
        __cEmitArgs(["(empty table)"]);
        return;
      }
      var colSet = Object.create(null);
      var cols = [];
      var hasValuesCol = false;
      var propFilter = Array.isArray(properties) ? properties.map(String) : null;
      for (var i = 0; i < rowKeys.length; i++) {
        var row = tabularData[rowKeys[i]];
        if (row !== null && typeof row === "object" && !(row instanceof Date) && !(row instanceof RegExp)) {
          var rks = Object.keys(row);
          for (var j = 0; j < rks.length; j++) {
            var ck = rks[j];
            if (propFilter && propFilter.indexOf(ck) === -1) continue;
            if (!colSet[ck]) {
              colSet[ck] = true;
              cols.push(ck);
            }
          }
        } else {
          hasValuesCol = true;
        }
      }
      if (hasValuesCol) cols.push("Values");
      var headers = ["(index)"].concat(cols);
      var widths = [];
      for (var h = 0; h < headers.length; h++) widths.push(headers[h].length);
      var matrix = [];
      for (var ri = 0; ri < rowKeys.length; ri++) {
        var rk = rowKeys[ri];
        var rv = tabularData[rk];
        var cells = [rk];
        if (rk.length > widths[0]) widths[0] = rk.length;
        var isObj = rv !== null && typeof rv === "object" && !(rv instanceof Date) && !(rv instanceof RegExp);
        for (var ci = 0; ci < cols.length; ci++) {
          var colName = cols[ci];
          var cellStr = "";
          if (colName === "Values" && !isObj) {
            cellStr = __cInsp(rv, 1, 1, []);
          } else if (isObj && Object.prototype.hasOwnProperty.call(rv, colName)) {
            cellStr = __cInsp(rv[colName], 1, 1, []);
          }
          cells.push(cellStr);
          if (cellStr.length > widths[ci + 1]) widths[ci + 1] = cellStr.length;
        }
        matrix.push(cells);
      }
      var sepParts = [];
      var hdrParts = [];
      for (var wi = 0; wi < headers.length; wi++) {
        var w = widths[wi];
        var dashes = "";
        for (var di = 0; di < w + 2; di++) dashes += "-";
        sepParts.push(dashes);
        hdrParts.push(" " + __cPad(headers[wi], w) + " ");
      }
      var border = "+" + sepParts.join("+") + "+";
      var lines = [border, "|" + hdrParts.join("|") + "|", border];
      for (var mi = 0; mi < matrix.length; mi++) {
        var rowCells = matrix[mi], rParts = [];
        for (var cj = 0; cj < rowCells.length; cj++) {
          rParts.push(" " + __cPad(rowCells[cj], widths[cj]) + " ");
        }
        lines.push("|" + rParts.join("|") + "|");
      }
      lines.push(border);
      __cEmitArgs([lines.join("\n")]);
    }
  };
})();

