#!/usr/bin/env python3
"""Serve a Wasm build as a host would: with the isolation headers a threaded
build requires (--isolated), or without them as GitHub Pages serves it, and
under a path prefix (--prefix /funfern/) as the site lives under one."""

import argparse
import sys
from functools import partial
from http import HTTPStatus
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer


class HostRequestHandler(SimpleHTTPRequestHandler):
    isolated = False
    prefix = "/"

    def end_headers(self) -> None:
        if self.isolated:
            self.send_header("Cross-Origin-Opener-Policy", "same-origin")
            self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        super().end_headers()

    def _under_prefix(self) -> bool:
        # Everything lives under the prefix; a request for anything else is as
        # absent as it is on the site.
        if self.prefix == "/":
            return True
        if self.path == self.prefix.rstrip("/"):
            self.path = self.prefix
        if not self.path.startswith(self.prefix):
            self.send_error(HTTPStatus.NOT_FOUND)
            return False
        self.path = "/" + self.path[len(self.prefix) :]
        return True

    def do_GET(self) -> None:
        if self._under_prefix():
            super().do_GET()

    def do_HEAD(self) -> None:
        if self._under_prefix():
            super().do_HEAD()


class HostServer(ThreadingHTTPServer):
    def handle_error(self, request, client_address) -> None:
        # A page that stops to install the isolation worker drops the module
        # it was downloading; the dropped connection is not an error here.
        if isinstance(sys.exc_info()[1], (BrokenPipeError, ConnectionResetError)):
            return
        super().handle_error(request, client_address)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--directory", required=True)
    parser.add_argument("--isolated", action="store_true")
    parser.add_argument("--prefix", default="/")
    args = parser.parse_args()
    if not args.prefix.startswith("/") or not args.prefix.endswith("/"):
        parser.error("--prefix starts and ends with a slash")
    HostRequestHandler.isolated = args.isolated
    HostRequestHandler.prefix = args.prefix
    handler = partial(HostRequestHandler, directory=args.directory)
    HostServer(("127.0.0.1", args.port), handler).serve_forever()


if __name__ == "__main__":
    main()
