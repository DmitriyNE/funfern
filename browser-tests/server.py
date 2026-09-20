#!/usr/bin/env python3
"""Serve a threaded Wasm build with the isolation headers it requires."""

import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer


class IsolatedRequestHandler(SimpleHTTPRequestHandler):
    def end_headers(self) -> None:
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        super().end_headers()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--directory", required=True)
    parser.add_argument("--isolated", action="store_true")
    args = parser.parse_args()
    handler_type = IsolatedRequestHandler if args.isolated else SimpleHTTPRequestHandler
    handler = partial(handler_type, directory=args.directory)
    ThreadingHTTPServer(("127.0.0.1", args.port), handler).serve_forever()


if __name__ == "__main__":
    main()
