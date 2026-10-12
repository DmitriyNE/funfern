#!/usr/bin/env bash
# Writes fixtures.html, the page that builds the browser bundle of the
# `funfern-fixtures` binary: index.html with its one rust link pointed at that
# binary and nothing else changed. Trunk takes the binary from the page's rust
# link alone, and writes the bundle's page as index.html whatever the source
# is called, so the fixture bundles keep the layout the tests serve. The file
# is generated and never committed (.gitignore), so it cannot drift from the
# page the app ships.
#
#   scripts/fixtures-page.sh && scripts/trunk build fixtures.html --release --dist dist-e2e
set -u
cd "$(dirname "$0")/.."
links=$(grep -c 'data-bin="funfern-app"' index.html)
if [[ $links -ne 1 ]]; then
  echo "index.html names funfern-app in $links rust links; expected one" >&2
  exit 1
fi
sed 's/data-bin="funfern-app"/data-bin="funfern-fixtures"/' index.html > fixtures.html
