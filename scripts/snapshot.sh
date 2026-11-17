#!/bin/sh
# Read-only listing of a folder: one line per file and folder with its size in
# bytes and modified time (to the nanosecond), sorted by path. Run it before and
# after a scan and diff the two outputs to prove the scan changed nothing.
#
#   scripts/snapshot.sh /Volumes/T7/Media > before.txt
#   ...scan in the app...
#   scripts/snapshot.sh /Volumes/T7/Media > after.txt
#   diff before.txt after.txt && echo IDENTICAL
#
# Write the output somewhere that is NOT on the drive being checked.
set -eu
[ $# -eq 1 ] && [ -d "$1" ] || { echo "usage: $0 <folder>" >&2; exit 2; }
cd "$1"
find . -print0 | xargs -0 stat -f '%N|%z|%Fm|%HT' | LC_ALL=C sort
