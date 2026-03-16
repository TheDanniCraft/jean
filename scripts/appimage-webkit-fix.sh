#!/usr/bin/env bash
# Custom AppRun: Fix WebKitGTK compatibility on rolling-release distros
#
# Problem: The default AppRun.wrapped binary hardcodes LD_LIBRARY_PATH to
# prioritize bundled Ubuntu 22.04 libraries. On newer distros (Arch, Fedora 40+,
# etc.), these bundled libraries conflict with system GPU drivers and Mesa,
# causing blank/white screens or crashes.
#
# Solution: Replace AppRun with a shell script that sources GTK hooks,
# sets LD_LIBRARY_PATH to prefer system libraries when system WebKitGTK
# is available, and then execs the Jean binary directly.
#
# Related issues:
# - https://github.com/coollabsio/jean/issues/52
# - https://github.com/coollabsio/jean/issues/55
# - https://github.com/coollabsio/jean/issues/71

set -eu

APPDIR="$(dirname "$(readlink -f "$0")")"
export APPDIR
ARCH_TRIPLET="$(uname -m)-linux-gnu"
BUNDLED_LIBS="$APPDIR/usr/lib:$APPDIR/usr/lib/$ARCH_TRIPLET:$APPDIR/usr/lib64:$APPDIR/lib:$APPDIR/lib/$ARCH_TRIPLET"
SYSTEM_LIBS="/usr/lib:/usr/lib64:/usr/lib/$ARCH_TRIPLET:/lib/$ARCH_TRIPLET"
EXISTING_LIBS="${LD_LIBRARY_PATH:-}"

# Source GTK plugin hooks (sets GDK_BACKEND, GTK_THEME, etc.)
for hook in "$APPDIR"/apprun-hooks/*.sh; do
    [ -f "$hook" ] && . "$hook"
done

# If system WebKitGTK 4.1 is available, prefer system libs first but keep bundled libs
# available as fallback so AppImage-provided libs still resolve when needed.
if [ -f /usr/lib/libwebkit2gtk-4.1.so.0 ] \
    || [ -f /usr/lib64/libwebkit2gtk-4.1.so.0 ] \
    || [ -f "/usr/lib/$ARCH_TRIPLET/libwebkit2gtk-4.1.so.0" ]; then
    export LD_LIBRARY_PATH="$SYSTEM_LIBS:$BUNDLED_LIBS"
else
    # Fallback: use bundled libraries (standard AppImage behavior)
    export LD_LIBRARY_PATH="$BUNDLED_LIBS"
fi

# Preserve any user-supplied LD_LIBRARY_PATH entries at the end.
if [ -n "$EXISTING_LIBS" ]; then
    export LD_LIBRARY_PATH="${LD_LIBRARY_PATH}:$EXISTING_LIBS"
fi

export PATH="$APPDIR/usr/bin:$PATH"
export XDG_DATA_DIRS="$APPDIR/usr/share:/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"

exec "$APPDIR/usr/bin/jean" "$@"
