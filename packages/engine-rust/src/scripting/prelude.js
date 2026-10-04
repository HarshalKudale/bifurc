(function () {
  var parse = JSON.parse;
  var stringify = JSON.stringify;
  var now = Date.now.bind(Date);
  var isArray = Array.isArray;
  var keys = Object.keys;
  var defineProperty = Object.defineProperty;
  var ErrorCtor = Error;
  var RegExpCtor = RegExp;
  var StringCtor = String;

  var input = parse(globalThis.__input);
  delete globalThis.__input;

  function copy(o) {
    var out = {};
    var k = keys(o);
    for (var i = 0; i < k.length; i++) out[k[i]] = o[k[i]];
    return out;
  }

  // Node runs a script in its own realm, where an error the script creates is not an instance of the host's
  // Error and is therefore reported as String(e) ("Error: boom"). Only errors the host raises — assertions
  // and syntax errors — are reported by their message alone.
  var hostErrors = new WeakSet();
  var isHostError = hostErrors.has.bind(hostErrors);
  var markHostError = hostErrors.add.bind(hostErrors);

  function fmt(e) {
    if (e instanceof ErrorCtor && (isHostError(e) || e.name === "SyntaxError")) return e.message;
    return StringCtor(e);
  }

  function buildExpect() {
    return function expect(actual) {
      function assertionError(msg) {
        var error = new ErrorCtor(msg);
        markHostError(error);
        return error;
      }

      var buildChain = function (negated) {
        var chain = {};
        var neg = negated ? "not " : "";

        function getter(name, fn) {
          defineProperty(chain, name, { get: fn });
        }

        var words = [
          "to", "be", "been", "is", "that", "which", "and", "has", "have", "with", "at", "of", "same", "but",
          "does", "still", "also",
        ];
        words.forEach(function (w) {
          getter(w, function () {
            return chain;
          });
        });

        getter("not", function () {
          return buildChain(!negated);
        });

        getter("ok", function () {
          var pass = !!actual;
          if (negated ? pass : !pass) {
            throw assertionError("expected " + stringify(actual) + " " + neg + "to be truthy");
          }
          return chain;
        });
        getter("true", function () {
          var pass = actual === true;
          if (negated ? pass : !pass) throw assertionError("expected " + stringify(actual) + " " + neg + "to be true");
          return chain;
        });
        getter("false", function () {
          var pass = actual === false;
          if (negated ? pass : !pass) throw assertionError("expected " + stringify(actual) + " " + neg + "to be false");
          return chain;
        });
        getter("null", function () {
          var pass = actual === null;
          if (negated ? pass : !pass) throw assertionError("expected " + stringify(actual) + " " + neg + "to be null");
          return chain;
        });
        getter("undefined", function () {
          var pass = actual === undefined;
          if (negated ? pass : !pass) {
            throw assertionError("expected " + stringify(actual) + " " + neg + "to be undefined");
          }
          return chain;
        });

        chain.equal = chain.equals = chain.eq = function (expected) {
          var pass = actual === expected;
          if (negated ? pass : !pass) {
            throw assertionError("expected " + stringify(actual) + " " + neg + "to equal " + stringify(expected));
          }
          return chain;
        };

        chain.eql = chain.eqls = function (expected) {
          var pass = stringify(actual) === stringify(expected);
          if (negated ? pass : !pass) {
            throw assertionError(
              "expected " + stringify(actual) + " " + neg + "to deeply equal " + stringify(expected),
            );
          }
          return chain;
        };

        chain.a = chain.an = function (type) {
          var pass = false;
          var t = type.toLowerCase();
          if (t === "array") pass = isArray(actual);
          else if (t === "object") pass = typeof actual === "object" && actual !== null && !isArray(actual);
          else if (t === "null") pass = actual === null;
          else pass = typeof actual === t;
          if (negated ? pass : !pass) {
            throw assertionError("expected " + stringify(actual) + " " + neg + "to be a(n) " + type);
          }
          return chain;
        };

        chain.include = chain.contain = chain.includes = chain.contains = function (val) {
          var pass = false;
          if (typeof actual === "string" && typeof val === "string") pass = actual.includes(val);
          else if (isArray(actual)) pass = actual.includes(val);
          else if (typeof actual === "object" && actual !== null && typeof val === "string") pass = val in actual;
          if (negated ? pass : !pass) {
            throw assertionError("expected " + stringify(actual) + " " + neg + "to include " + stringify(val));
          }
          return chain;
        };

        chain.property = function (key) {
          var pass = typeof actual === "object" && actual !== null && key in actual;
          if (negated ? pass : !pass) {
            throw assertionError("expected object " + neg + 'to have property "' + key + '"');
          }
          return chain;
        };

        chain.length = chain.lengthOf = function (n) {
          var len = actual == null ? undefined : actual.length;
          var pass = len === n;
          if (negated ? pass : !pass) {
            throw assertionError("expected length " + len + " " + neg + "to equal " + n);
          }
          return chain;
        };

        chain.lengthAbove = function (n) {
          var len = actual == null ? undefined : actual.length;
          var pass = typeof len === "number" && len > n;
          if (negated ? pass : !pass) {
            throw assertionError("expected length " + len + " " + neg + "to be above " + n);
          }
          return chain;
        };

        chain.above = chain.greaterThan = chain.gt = function (n) {
          var pass = typeof actual === "number" && actual > n;
          if (negated ? pass : !pass) throw assertionError("expected " + actual + " " + neg + "to be above " + n);
          return chain;
        };

        chain.below = chain.lessThan = chain.lt = function (n) {
          var pass = typeof actual === "number" && actual < n;
          if (negated ? pass : !pass) throw assertionError("expected " + actual + " " + neg + "to be below " + n);
          return chain;
        };

        chain.least = chain.gte = function (n) {
          var pass = typeof actual === "number" && actual >= n;
          if (negated ? pass : !pass) throw assertionError("expected " + actual + " " + neg + "to be at least " + n);
          return chain;
        };

        chain.most = chain.lte = function (n) {
          var pass = typeof actual === "number" && actual <= n;
          if (negated ? pass : !pass) throw assertionError("expected " + actual + " " + neg + "to be at most " + n);
          return chain;
        };

        chain.match = chain.matches = function (re) {
          var regex = typeof re === "string" ? new RegExpCtor(re) : re;
          var pass = typeof actual === "string" && regex.test(actual);
          if (negated ? pass : !pass) {
            throw assertionError('expected "' + actual + '" ' + neg + "to match " + regex);
          }
          return chain;
        };

        chain.status = function (code) {
          var pass = actual === code;
          if (negated ? pass : !pass) {
            throw assertionError("expected status " + actual + " " + neg + "to be " + code);
          }
          return chain;
        };

        getter("empty", function () {
          var pass = false;
          if (typeof actual === "string" || isArray(actual)) pass = actual.length === 0;
          else if (typeof actual === "object" && actual !== null) pass = keys(actual).length === 0;
          if (negated ? pass : !pass) {
            throw assertionError("expected " + stringify(actual) + " " + neg + "to be empty");
          }
          return chain;
        });

        chain.oneOf = function (list) {
          var pass = list.includes(actual);
          if (negated ? pass : !pass) {
            throw assertionError("expected " + stringify(actual) + " " + neg + "to be one of " + stringify(list));
          }
          return chain;
        };

        return chain;
      };

      return buildChain(false);
    };
  }

  function buildConsoleMock(logs) {
    function join(args) {
      return args
        .map(function (a) {
          return typeof a === "string" ? a : stringify(a);
        })
        .join(" ");
    }
    return {
      log: function () {
        logs.push(join([].slice.call(arguments)));
      },
      warn: function () {
        logs.push("[WARN] " + join([].slice.call(arguments)));
      },
      error: function () {
        logs.push("[ERROR] " + join([].slice.call(arguments)));
      },
      info: function () {
        logs.push(join([].slice.call(arguments)));
      },
    };
  }

  var mode = input.mode;
  var vars = copy(input.envVars || {});
  var result = null;
  var tests = [];
  var logs = [];

  function findKey(obj, key) {
    if (key in obj) return key;
    var lower = StringCtor(key).toLowerCase();
    var k = keys(obj);
    for (var i = 0; i < k.length; i++) if (k[i].toLowerCase() === lower) return k[i];
    return undefined;
  }

  // Header accessors, case-insensitive like HTTP itself. `store` is read live so a mutation shows up.
  function headerApi(getStore, writable) {
    var api = {
      get: function (key) {
        var store = getStore();
        var found = findKey(store, key);
        return found === undefined ? undefined : store[found];
      },
      has: function (key) {
        return findKey(getStore(), key) !== undefined;
      },
      toObject: function () {
        return copy(getStore());
      },
    };
    if (writable) {
      api.set = function (key, value) {
        var store = getStore();
        var found = findKey(store, key);
        if (found !== undefined && found !== key) delete store[found];
        store[key] = StringCtor(value);
      };
      api.unset = function (key) {
        var store = getStore();
        var found = findKey(store, key);
        if (found !== undefined) delete store[found];
      };
    }
    return api;
  }

  function responseApi(res) {
    return {
      get status() {
        return res.status;
      },
      get body() {
        return res.body;
      },
      get responseTime() {
        return res.responseTime == null ? 0 : res.responseTime;
      },
      headers: headerApi(function () {
        return res.headers;
      }, false),
      json: function () {
        try {
          return parse(res.body);
        } catch (e) {
          return null;
        }
      },
      text: function () {
        return res.body;
      },
    };
  }

  var envApi = {
    get: function (key) {
      return vars[key];
    },
    set: function (key, value) {
      vars[key] = StringCtor(value);
    },
    unset: function (key) {
      delete vars[key];
    },
    has: function (key) {
      return key in vars;
    },
    toObject: function () {
      return copy(vars);
    },
  };

  var consoleApi = buildConsoleMock(logs);
  var expectApi = buildExpect();

  if (mode === "proxyRequest" || mode === "proxyResponse") {
    var mutableHeaders = copy(input.headers);
    var mutableBody = input.body;
    var side = {
      get headers() {
        return mutableHeaders;
      },
      get body() {
        return mutableBody;
      },
      set body(v) {
        mutableBody = v;
      },
    };
    var proxyLp = {};
    proxyLp[mode === "proxyRequest" ? "request" : "response"] = side;
    globalThis.lp = proxyLp;
    result = function (err) {
      var out = { headers: mutableHeaders, body: typeof mutableBody === "string" ? mutableBody : StringCtor(mutableBody) };
      if (err !== undefined) out.error = err;
      return out;
    };
  } else {
    var req = copy(input.request || {});
    req.headers = copy((input.request && input.request.headers) || {});
    var res = input.response ? copy(input.response) : null;
    if (res) res.headers = copy(input.response.headers || {});
    var writable = mode === "pre";

    var requestApi = {
      get url() {
        return req.url;
      },
      get method() {
        return req.method;
      },
      get body() {
        return req.body;
      },
      headers: headerApi(function () {
        return req.headers;
      }, writable),
    };
    if (writable) {
      defineProperty(requestApi, "url", {
        get: function () {
          return req.url;
        },
        set: function (v) {
          req.url = v;
        },
        enumerable: true,
      });
      defineProperty(requestApi, "method", {
        get: function () {
          return req.method;
        },
        set: function (v) {
          req.method = v;
        },
        enumerable: true,
      });
      defineProperty(requestApi, "body", {
        get: function () {
          return req.body;
        },
        set: function (v) {
          req.body = v;
        },
        enumerable: true,
      });
    }

    var lp = { request: requestApi, environment: envApi };
    if (res) lp.response = responseApi(res);
    lp.test = function (name, fn) {
      var start = now();
      try {
        fn();
        tests.push({ name: name, passed: true, durationMs: now() - start });
      } catch (e) {
        tests.push({ name: name, passed: false, error: fmt(e), durationMs: now() - start });
      }
    };
    lp.expect = expectApi;
    lp.console = consoleApi;
    globalThis.lp = lp;
    globalThis.console = consoleApi;
    globalThis.test = lp.test;
    globalThis.expect = expectApi;

    result = function (err) {
      if (mode === "test") {
        if (err !== undefined) {
          if (tests.length === 0) {
            tests.push({ name: "Script execution", passed: false, error: err, durationMs: 0 });
          } else {
            logs.push("[ERROR] Uncaught: " + err);
          }
        }
        return { envVars: vars, testResults: tests, testLogs: logs };
      }
      var out = { envVars: vars, logs: logs };
      if (mode === "pre") out.request = req;
      else out.response = res;
      if (err !== undefined) out.error = err;
      return out;
    };
  }

  return {
    fmt: fmt,
    collect: function (err) {
      return stringify(result(err));
    },
  };
})()
