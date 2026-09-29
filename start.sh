#!/bin/sh
cd "$(dirname "$0")" && exec bun server.js "$@"
