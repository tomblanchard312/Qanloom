#!/bin/sh
# deb/rpm post-install and post-remove: on rpm, install or update the shared Enki from the
# bundled payload (the .deb depends on the enki-runtime package instead and ships no
# payload, so this step is skipped there), then refresh the desktop, MIME and icon caches. Every cache tool is optional; missing ones are skipped.
# deb passes "configure"; rpm postinstall passes 1 (install) or 2 (upgrade).
case "${1:-configure}" in
  configure | 1 | 2)
    payload=/usr/lib/nisaba/enki-payload
    if [ -x "$payload/enki-install" ]; then
      "$payload/enki-install" --payload "$payload" --prefix /usr/lib/enki
    fi
    ;;
esac
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q /usr/share/applications || true
fi
if command -v update-mime-database >/dev/null 2>&1; then
  update-mime-database /usr/share/mime || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi
exit 0
