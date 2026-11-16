// MAIN world, document_start. Wraps fetch/XHR so the page's OWN GraphQL
// responses can be read — the page makes (and signs) the requests itself, so we
// never forge a call and X rotating its queryId / transaction-id can't break
// us. Only graphql responses are forwarded; everything else passes through
// untouched. Captures go to x-relay.js (isolated world) via window.postMessage,
// because MAIN world can't use chrome.* APIs.
(function () {
  const TAG = "linggen-x-hook";

  function opName(url) {
    const m = String(url).match(/\/graphql\/[^/]+\/([A-Za-z0-9_]+)/);
    return m ? m[1] : null;
  }

  function report(url, data) {
    const op = opName(url);
    if (!op || data == null) return;
    try {
      window.postMessage({ source: TAG, op, data }, window.location.origin);
    } catch (e) {
      /* ignore */
    }
  }

  const origFetch = window.fetch;
  if (origFetch) {
    window.fetch = function (...args) {
      const promise = origFetch.apply(this, args);
      promise
        .then((res) => {
          try {
            const url =
              (typeof args[0] === "string" ? args[0] : args[0] && args[0].url) ||
              res.url ||
              "";
            if (url.indexOf("/graphql/") !== -1) {
              res.clone().json().then((data) => report(url, data)).catch(() => {});
            }
          } catch (e) {
            /* ignore */
          }
        })
        .catch(() => {});
      return promise;
    };
  }

  const origOpen = XMLHttpRequest.prototype.open;
  const origSend = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function (method, url) {
    this.__lgUrl = url;
    return origOpen.apply(this, arguments);
  };
  XMLHttpRequest.prototype.send = function () {
    this.addEventListener("load", () => {
      try {
        const url = this.__lgUrl || "";
        if (url.indexOf("/graphql/") !== -1) {
          report(url, JSON.parse(this.responseText));
        }
      } catch (e) {
        /* ignore */
      }
    });
    return origSend.apply(this, arguments);
  };
})();
