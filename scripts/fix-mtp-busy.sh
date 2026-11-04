#!/usr/bin/env bash
# Frees MTP devices (e.g. a Kindle) that GVFS or KDE's kiod/kio-mtp have
# auto-claimed over USB, which makes libusb_claim_interface() fail with
# "device busy" for mtp-filler (and other libmtp-based tools).
#
# mtp-filler now runs this same recovery automatically when a device open
# fails, so you shouldn't normally need to run this by hand. It's kept here
# as a standalone fallback (e.g. to free the device for a different tool).
#
# Usage: ./scripts/fix-mtp-busy.sh
# Run as the same user (or root) that owns the process holding the device;
# root can kill regardless of owner.

set -uo pipefail

killed_any=0

for pid_dir in /proc/[0-9]*; do
    pid=${pid_dir#/proc/}
    [ "$pid" = "$$" ] && continue

    comm_file="$pid_dir/comm"
    [ -r "$comm_file" ] || continue
    comm=$(<"$comm_file")

    case "$comm" in
        *gvfs*mtp* | *mtp*gvfs* | kiod* | kio_mtp* | *gphoto2*) ;;
        *) continue ;;
    esac

    fd_dir="$pid_dir/fd"
    [ -d "$fd_dir" ] || continue

    holds_usb=0
    for fd in "$fd_dir"/*; do
        [ -e "$fd" ] || continue
        target=$(readlink "$fd" 2>/dev/null || true)
        case "$target" in
            /dev/bus/usb/*)
                holds_usb=1
                break
                ;;
        esac
    done

    if [ "$holds_usb" = 1 ]; then
        echo "Killing '$comm' (pid $pid), which is holding an MTP/USB device open"
        kill "$pid" 2>/dev/null
        killed_any=1
    fi
done

if [ "$killed_any" = 1 ]; then
    echo "Freed MTP device(s). You can retry mtp-filler now."
else
    echo "No MTP-related process appears to be holding a USB device open."
fi
