#!/bin/sh
set -e
cd "$(dirname "$0")"
ffmpeg -y -loglevel error -f lavfi -i "sine=frequency=440:duration=1:sample_rate=44100" -ac 1 -c:a aac -b:a 64k sample.m4a
ffmpeg -y -loglevel error -f lavfi -i "testsrc=duration=1:size=64x64:rate=10" -f lavfi -i "sine=frequency=440:duration=1:sample_rate=44100" -ac 1 -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest sample.mp4
