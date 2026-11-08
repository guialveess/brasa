"use strict";

// Embedded prelude for the Postino script sandbox.
//
// This file builds the whole JavaScript-facing API (req, res, vars, env, test, expect,
// console) on top of a single plain data object, `globalThis.__input`, set by Rust before this
// file is evaluated. It never touches the network, the filesystem or a clock: everything here is
// pure JavaScript operating on data already in memory. The only host calls left to Rust are the
// `util.*` functions, installed separately as native bindings before this file runs.
//
// After the user script has run, Rust evaluates `JSON.stringify(__collect())` to read back the
// final state (mutated request, vars, environment changes, console lines, test results).

(function () {
    var input = globalThis.__input;
    var isPost = input.response !== null;

    // A flat list of { key, value } entries, the JS-side mirror of a `Vec<KeyValue>`.
    function toEntries(list) {
        var entries = [];
        for (var i = 0; i < list.length; i++) {
            entries.push({ key: list[i].key, value: list[i].value });
        }
        return entries;
    }

    // Whether two entry names are the same. Header names are case insensitive, variable names
    // are not, so only the header lists pass `fold = true`.
    function sameName(a, b, fold) {
        return fold ? a.toLowerCase() === b.toLowerCase() : a === b;
    }

    function entriesGet(entries, name, fold) {
        for (var i = 0; i < entries.length; i++) {
            if (sameName(entries[i].key, name, fold)) {
                return entries[i].value;
            }
        }
        return undefined;
    }

    // An existing entry keeps the spelling of its name and only gets the new value.
    function entriesSet(entries, name, value, fold) {
        var text = String(value);
        for (var i = 0; i < entries.length; i++) {
            if (sameName(entries[i].key, name, fold)) {
                entries[i].value = text;
                return;
            }
        }
        entries.push({ key: name, value: text });
    }

    function entriesRemove(entries, name, fold) {
        for (var i = entries.length - 1; i >= 0; i--) {
            if (sameName(entries[i].key, name, fold)) {
                entries.splice(i, 1);
            }
        }
    }

    // --- req ---------------------------------------------------------------------------------

    var reqHeaders = toEntries(input.request.headers);
    var reqState = {
        method: input.request.method,
        url: input.request.url,
        body: input.request.body,
    };

    function makeHeaders(entries, readOnly) {
        return {
            get: function (name) {
                return entriesGet(entries, name, true);
            },
            set: function (name, value) {
                if (readOnly) {
                    throw new Error("headers are read-only in a post script");
                }
                entriesSet(entries, name, value, true);
            },
            remove: function (name) {
                if (readOnly) {
                    throw new Error("headers are read-only in a post script");
                }
                entriesRemove(entries, name, true);
            },
        };
    }

    var req = {
        headers: makeHeaders(reqHeaders, isPost),
    };
    Object.defineProperty(req, "method", {
        enumerable: true,
        get: function () {
            return reqState.method;
        },
        set: function (value) {
            if (isPost) {
                throw new Error("req.method is read-only in a post script");
            }
            reqState.method = String(value);
        },
    });
    Object.defineProperty(req, "url", {
        enumerable: true,
        get: function () {
            return reqState.url;
        },
        set: function (value) {
            if (isPost) {
                throw new Error("req.url is read-only in a post script");
            }
            reqState.url = String(value);
        },
    });
    Object.defineProperty(req, "body", {
        enumerable: true,
        get: function () {
            return reqState.body;
        },
        set: function (value) {
            if (isPost) {
                throw new Error("req.body is read-only in a post script");
            }
            reqState.body = String(value);
        },
    });
    globalThis.req = req;

    // --- res (post only) ----------------------------------------------------------------------

    if (isPost) {
        var resHeaders = toEntries(input.response.headers);
        globalThis.res = {
            status: input.response.status,
            headers: makeHeaders(resHeaders, true),
            body: input.response.body,
            timeMs: input.response.time_ms,
            size: input.response.size,
            json: function () {
                return JSON.parse(input.response.body);
            },
        };
    }

    // --- vars ----------------------------------------------------------------------------------

    var varsEntries = toEntries(input.vars);
    globalThis.vars = {
        get: function (name) {
            return entriesGet(varsEntries, name);
        },
        set: function (name, value) {
            entriesSet(varsEntries, name, value);
        },
    };

    // --- env -----------------------------------------------------------------------------------

    var envEntries = toEntries(input.env);
    var envChanges = [];
    globalThis.env = {
        get: function (name) {
            return entriesGet(envEntries, name);
        },
        set: function (name, value) {
            var text = String(value);
            entriesSet(envEntries, name, text);
            envChanges.push({ type: "set", key: name, value: text });
        },
        unset: function (name) {
            entriesRemove(envEntries, name);
            envChanges.push({ type: "unset", key: name });
        },
    };

    // --- console ---------------------------------------------------------------------------------

    var consoleLines = [];

    function describeForLog(value) {
        if (typeof value === "string") {
            return value;
        }
        try {
            var text = JSON.stringify(value);
            return text === undefined ? String(value) : text;
        } catch (error) {
            return String(value);
        }
    }

    // The line of the user script that called `console.*`: the first stack frame in the script
    // itself (`pre-script` or `post-script`, the names `quickjs_engine.rs` evaluates it under).
    function callerLine() {
        try {
            var match = /(?:pre|post)-script:(\d+)/.exec(new Error().stack || "");
            return match ? Number(match[1]) : null;
        } catch (error) {
            return null;
        }
    }

    function makeLog(level) {
        return function () {
            var parts = [];
            for (var i = 0; i < arguments.length; i++) {
                parts.push(describeForLog(arguments[i]));
            }
            consoleLines.push({
                level: level,
                text: parts.join(" "),
                time: Date.now(),
                line: callerLine(),
            });
        };
    }

    globalThis.console = {
        log: makeLog("log"),
        info: makeLog("info"),
        warn: makeLog("warn"),
        error: makeLog("error"),
    };

    // --- test / expect -------------------------------------------------------------------------

    var testResults = [];

    function describeValue(value) {
        if (typeof value === "string") {
            return JSON.stringify(value);
        }
        try {
            var text = JSON.stringify(value);
            return text === undefined ? String(value) : text;
        } catch (error) {
            return String(value);
        }
    }

    function deepEqual(a, b) {
        return JSON.stringify(a) === JSON.stringify(b);
    }

    function makeMatchers(value, negate) {
        function report(conditionHolds, message) {
            var shouldThrow = negate ? conditionHolds : !conditionHolds;
            if (shouldThrow) {
                throw new Error(message);
            }
        }
        var not = negate ? " not" : "";
        var matchers = {
            toBe: function (expected) {
                report(
                    value === expected,
                    "expected " + describeValue(value) + not + " to be " + describeValue(expected)
                );
            },
            toEqual: function (expected) {
                report(
                    deepEqual(value, expected),
                    "expected " + describeValue(value) + not + " to equal " + describeValue(expected)
                );
            },
            toBeTruthy: function () {
                report(Boolean(value), "expected " + describeValue(value) + not + " to be truthy");
            },
            toBeFalsy: function () {
                report(!value, "expected " + describeValue(value) + not + " to be falsy");
            },
            toContain: function (item) {
                var has;
                if (typeof value === "string") {
                    has = value.indexOf(item) !== -1;
                } else if (Array.isArray(value)) {
                    has = value.indexOf(item) !== -1;
                } else {
                    throw new Error("toContain needs a string or an array, got " + describeValue(value));
                }
                report(has, "expected " + describeValue(value) + not + " to contain " + describeValue(item));
            },
            toMatch: function (pattern) {
                var re = pattern instanceof RegExp ? pattern : new RegExp(pattern);
                report(
                    re.test(String(value)),
                    "expected " + describeValue(value) + not + " to match " + String(pattern)
                );
            },
            toBeGreaterThan: function (other) {
                report(
                    value > other,
                    "expected " + describeValue(value) + not + " to be greater than " + describeValue(other)
                );
            },
            toBeLessThan: function (other) {
                report(
                    value < other,
                    "expected " + describeValue(value) + not + " to be less than " + describeValue(other)
                );
            },
            toHaveProperty: function (name) {
                var has =
                    value !== null &&
                    value !== undefined &&
                    Object.prototype.hasOwnProperty.call(Object(value), name);
                report(
                    has,
                    "expected " + describeValue(value) + not + " to have property " + JSON.stringify(name)
                );
            },
        };
        Object.defineProperty(matchers, "not", {
            get: function () {
                return makeMatchers(value, !negate);
            },
        });
        return matchers;
    }

    globalThis.expect = function (value) {
        return makeMatchers(value, false);
    };

    globalThis.test = function (name, fn) {
        var testName = String(name);
        try {
            fn();
            testResults.push({ name: testName, passed: true, message: null });
        } catch (error) {
            var message = error && error.message !== undefined ? error.message : String(error);
            testResults.push({ name: testName, passed: false, message: String(message) });
        }
    };

    // --- collect ---------------------------------------------------------------------------------

    globalThis.__collect = function () {
        var out = {
            request: null,
            vars: varsEntries,
            env_changes: envChanges,
            tests: testResults,
            console: consoleLines,
        };
        if (!isPost) {
            out.request = {
                method: reqState.method,
                url: reqState.url,
                headers: reqHeaders,
                body: reqState.body,
            };
        }
        return out;
    };
})();
