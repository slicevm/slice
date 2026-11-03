// ============================================================================
// SHARD 9: WEBASSEMBLY (W3C / WinterTC Standard API Backed by Pure-Rust wasmi)
// ============================================================================
(function() {
  function __makeWasmErrorCtor(name) {
    function WasmErr(message) {
      var err = new Error(message !== undefined ? String(message) : "");
      err.name = name;
      Object.setPrototypeOf(err, WasmErr.prototype);
      return err;
    }
    WasmErr.prototype = Object.create(Error.prototype);
    WasmErr.prototype.constructor = WasmErr;
    WasmErr.prototype.name = name;
    return WasmErr;
  }

  var CompileError = __makeWasmErrorCtor("CompileError");
  var LinkError = __makeWasmErrorCtor("LinkError");
  var RuntimeError = __makeWasmErrorCtor("RuntimeError");

  function __rethrowWasmError(e) {
    var msg = e && e.message ? String(e.message) : String(e);
    if (msg.indexOf("CompileError:") !== -1) {
      throw new CompileError(msg.replace(/^.*CompileError:\s*/, ""));
    }
    if (msg.indexOf("LinkError:") !== -1) {
      throw new LinkError(msg.replace(/^.*LinkError:\s*/, ""));
    }
    if (msg.indexOf("RuntimeError:") !== -1) {
      throw new RuntimeError(msg.replace(/^.*RuntimeError:\s*/, ""));
    }
    if (msg.indexOf("RangeError:") !== -1) {
      throw new RangeError(msg.replace(/^.*RangeError:\s*/, ""));
    }
    if (msg.indexOf("TypeError:") !== -1) {
      throw new TypeError(msg.replace(/^.*TypeError:\s*/, ""));
    }
    throw e;
  }

  function Memory(descriptor) {
    if (!descriptor || typeof descriptor !== "object") {
      throw new TypeError("WebAssembly.Memory(): Descriptor must be an object");
    }
    var initPages = descriptor.initial !== undefined ? descriptor.initial : descriptor.minimum;
    if (initPages === undefined || typeof initPages !== "number" || initPages < 0) {
      throw new TypeError("WebAssembly.Memory(): Property 'initial' is required and must be a non-negative number");
    }
    initPages = initPages >>> 0;
    var maxPages = descriptor.maximum !== undefined ? (descriptor.maximum >>> 0) : undefined;
    if (descriptor._instId) {
      this._memId = 0;
      this._instId = descriptor._instId;
      this._pages = initPages;
      this._maxPages = maxPages;
      this._buf = new ArrayBuffer(initPages * 65536);
      try {
        __slice_wasm_sync_init_mem(this._instId, this._buf);
      } catch (e) {
        __rethrowWasmError(e);
      }
      return;
    }
    try {
      this._memId = __slice_wasm_memory_create(initPages, maxPages);
    } catch (e) {
      __rethrowWasmError(e);
    }
    this._instId = 0;
    this._pages = initPages;
    this._maxPages = maxPages;
    this._buf = new ArrayBuffer(initPages * 65536);
  }

  Object.defineProperty(Memory.prototype, "buffer", {
    get: function() {
      return this._buf;
    },
    enumerable: true,
    configurable: true
  });

  Memory.prototype.grow = function(delta) {
    var d = (delta === undefined ? 0 : Number(delta)) >>> 0;
    var prevPages = this._pages;
    try {
      prevPages = __slice_wasm_memory_grow(this._memId || 0, this._instId || 0, d);
    } catch (e) {
      __rethrowWasmError(e);
    }
    if (d > 0) {
      var newPages = prevPages + d;
      var newBuf = new ArrayBuffer(newPages * 65536);
      new Uint8Array(newBuf).set(new Uint8Array(this._buf));
      this._buf = newBuf;
      this._pages = newPages;
    }
    return prevPages;
  };

  function Global(descriptor, v) {
    if (!descriptor || typeof descriptor !== "object") {
      throw new TypeError("WebAssembly.Global(): Descriptor must be an object");
    }
    this._type = String(descriptor.value || "i32");
    this._mutable = Boolean(descriptor.mutable);
    this._instId = descriptor._instId || 0;
    this._exportName = descriptor._exportName || "";
    if (!this._instId) {
      if (this._type === "i64") {
        this._val = v !== undefined ? BigInt(v) : 0n;
      } else {
        this._val = v !== undefined ? Number(v) : 0;
      }
    }
  }

  Object.defineProperty(Global.prototype, "value", {
    get: function() {
      if (this._instId) {
        return __slice_wasm_global_get(this._instId, this._exportName);
      }
      return this._val;
    },
    set: function(nv) {
      if (!this._mutable) {
        throw new TypeError("WebAssembly.Global.value: cannot change value of an immutable global");
      }
      if (this._instId) {
        try {
          __slice_wasm_global_set(this._instId, this._exportName, nv);
        } catch (e) {
          __rethrowWasmError(e);
        }
        return;
      }
      this._val = this._type === "i64" ? BigInt(nv) : Number(nv);
    },
    enumerable: true,
    configurable: true
  });

  Global.prototype.valueOf = function() {
    return this.value;
  };

  function Table(descriptor, fill) {
    if (!descriptor || typeof descriptor !== "object") {
      throw new TypeError("WebAssembly.Table(): Descriptor must be an object");
    }
    var initLen = descriptor.initial !== undefined ? descriptor.initial : descriptor.minimum;
    if (initLen === undefined || typeof initLen !== "number" || initLen < 0) {
      throw new TypeError("WebAssembly.Table(): Property 'initial' is required");
    }
    this._element = String(descriptor.element || "funcref");
    this._max = descriptor.maximum !== undefined ? (descriptor.maximum >>> 0) : undefined;
    var n = initLen >>> 0;
    if (this._max !== undefined && n > this._max) {
      throw new RangeError("WebAssembly.Table(): initial length exceeds maximum");
    }
    this._items = new Array(n);
    var initVal = fill !== undefined ? fill : null;
    for (var i = 0; i < n; i++) this._items[i] = initVal;
  }

  Object.defineProperty(Table.prototype, "length", {
    get: function() {
      return this._items.length;
    },
    enumerable: true,
    configurable: true
  });

  Table.prototype.get = function(idx) {
    var i = idx >>> 0;
    if (i >= this._items.length) {
      throw new RangeError("WebAssembly.Table.get(): index out of bounds");
    }
    return this._items[i];
  };

  Table.prototype.set = function(idx, val) {
    var i = idx >>> 0;
    if (i >= this._items.length) {
      throw new RangeError("WebAssembly.Table.set(): index out of bounds");
    }
    this._items[i] = val !== undefined ? val : null;
  };

  Table.prototype.grow = function(delta, fill) {
    var d = (delta === undefined ? 0 : Number(delta)) >>> 0;
    var oldLen = this._items.length;
    var newLen = oldLen + d;
    if (this._max !== undefined && newLen > this._max) {
      throw new RangeError("WebAssembly.Table.grow(): maximum table length exceeded");
    }
    var fillVal = fill !== undefined ? fill : null;
    for (var i = 0; i < d; i++) {
      this._items.push(fillVal);
    }
    return oldLen;
  };

  function Module(bytes) {
    if (!(this instanceof Module)) {
      throw new TypeError("WebAssembly.Module must be called with new");
    }
    if (!bytes || (typeof bytes !== "object" && typeof bytes !== "function")) {
      throw new TypeError("WebAssembly.Module(): Argument 0 must be a buffer source");
    }
    try {
      this._modId = __slice_wasm_compile(bytes);
    } catch (e) {
      __rethrowWasmError(e);
    }
  }

  Module.exports = function(mod) {
    if (!(mod instanceof Module) || typeof mod._modId !== "number") {
      throw new TypeError("WebAssembly.Module.exports(): Argument 0 must be a WebAssembly.Module");
    }
    return JSON.parse(__slice_wasm_module_exports(mod._modId));
  };

  Module.imports = function(mod) {
    if (!(mod instanceof Module) || typeof mod._modId !== "number") {
      throw new TypeError("WebAssembly.Module.imports(): Argument 0 must be a WebAssembly.Module");
    }
    return JSON.parse(__slice_wasm_module_imports(mod._modId));
  };

  Module.customSections = function(mod, sectionName) {
    if (!(mod instanceof Module) || typeof mod._modId !== "number") {
      throw new TypeError("WebAssembly.Module.customSections(): Argument 0 must be a WebAssembly.Module");
    }
    if (arguments.length < 2) {
      throw new TypeError("WebAssembly.Module.customSections(): Argument 1 is required");
    }
    return __slice_wasm_module_custom_sections(mod._modId, String(sectionName));
  };

  function Instance(mod, importObject) {
    if (!(this instanceof Instance)) {
      throw new TypeError("WebAssembly.Instance must be called with new");
    }
    if (!(mod instanceof Module) || typeof mod._modId !== "number") {
      throw new TypeError("WebAssembly.Instance(): Argument 0 must be a WebAssembly.Module");
    }
    if (importObject !== undefined && (typeof importObject !== "object" || importObject === null)) {
      throw new TypeError("WebAssembly.Instance(): Argument 1 must be an object or undefined");
    }
    var importedMemObj = null;
    if (importObject) {
      var modKeys = Object.keys(importObject);
      for (var k = 0; k < modKeys.length; k++) {
        var ns = importObject[modKeys[k]];
        if (ns && typeof ns === "object") {
          var fKeys = Object.keys(ns);
          for (var j = 0; j < fKeys.length; j++) {
            if (ns[fKeys[j]] instanceof Memory) {
              importedMemObj = ns[fKeys[j]];
            }
          }
        }
      }
    }
    var meta;
    try {
      meta = __slice_wasm_instantiate(mod._modId, importObject);
    } catch (e) {
      __rethrowWasmError(e);
    }
    var instId = meta.instId;
    this._instId = instId;
    var activeMemObj = importedMemObj;
    if (activeMemObj) {
      activeMemObj._instId = instId;
    }

    var expList = JSON.parse(meta.exportsJson || "[]");
    var exportsObj = {};

    for (var i = 0; i < expList.length; i++) {
      var item = expList[i];
      if (item.kind === "memory") {
        if (!activeMemObj) {
          activeMemObj = new Memory({
            initial: item.pages,
            maximum: item.maxPages !== null ? item.maxPages : undefined,
            _instId: instId
          });
        }
        exportsObj[item.name] = activeMemObj;
      }
    }

    for (var idx = 0; idx < expList.length; idx++) {
      var exp = expList[idx];
      if (exp.kind === "function") {
        (function(fnName, arity) {
          var wrapper = function() {
            var callArgs = [];
            for (var a = 0; a < arguments.length; a++) {
              callArgs.push(arguments[a]);
            }
            try {
              return __slice_wasm_call(
                instId,
                fnName,
                callArgs,
                activeMemObj ? activeMemObj._buf : null
              );
            } catch (e) {
              __rethrowWasmError(e);
            }
          };
          try {
            Object.defineProperty(wrapper, "length", { value: arity, configurable: true });
            Object.defineProperty(wrapper, "name", { value: fnName, configurable: true });
          } catch (_) {}
          exportsObj[fnName] = wrapper;
        })(exp.name, exp.arity || 0);
      } else if (exp.kind === "global") {
        exportsObj[exp.name] = new Global({
          value: exp.isI64 ? "i64" : "f64",
          mutable: Boolean(exp.mutable),
          _instId: instId,
          _exportName: exp.name
        });
      } else if (exp.kind === "table") {
        exportsObj[exp.name] = new Table({
          element: "funcref",
          initial: exp.length || 0
        });
      }
    }

    this.exports = Object.freeze(exportsObj);
  }

  function validate(bytes) {
    if (!bytes || (typeof bytes !== "object" && typeof bytes !== "function")) {
      throw new TypeError("WebAssembly.validate(): Argument 0 must be a buffer source");
    }
    try {
      return Boolean(__slice_wasm_validate(bytes));
    } catch (_) {
      return false;
    }
  }

  function compile(bytes) {
    return new Promise(function(resolve, reject) {
      try {
        resolve(new Module(bytes));
      } catch (e) {
        reject(e);
      }
    });
  }

  function instantiate(bytesOrModule, importObject) {
    return new Promise(function(resolve, reject) {
      try {
        if (bytesOrModule instanceof Module) {
          resolve(new Instance(bytesOrModule, importObject));
        } else {
          var mod = new Module(bytesOrModule);
          var inst = new Instance(mod, importObject);
          resolve({ module: mod, instance: inst });
        }
      } catch (e) {
        reject(e);
      }
    });
  }

  function compileStreaming(source) {
    return Promise.resolve(source)
      .then(function(resp) {
        if (resp && typeof resp.arrayBuffer === "function") {
          return resp.arrayBuffer();
        }
        return resp;
      })
      .then(function(buf) {
        return compile(buf);
      });
  }

  function instantiateStreaming(source, importObject) {
    return Promise.resolve(source)
      .then(function(resp) {
        if (resp && typeof resp.arrayBuffer === "function") {
          return resp.arrayBuffer();
        }
        return resp;
      })
      .then(function(buf) {
        return instantiate(buf, importObject);
      });
  }

  globalThis.WebAssembly = {
    Module: Module,
    Instance: Instance,
    Memory: Memory,
    Global: Global,
    Table: Table,
    CompileError: CompileError,
    LinkError: LinkError,
    RuntimeError: RuntimeError,
    validate: validate,
    compile: compile,
    instantiate: instantiate,
    compileStreaming: compileStreaming,
    instantiateStreaming: instantiateStreaming
  };
})();

