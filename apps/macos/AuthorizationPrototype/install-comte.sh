#!/bin/bash
# Explicitly invoked by an administrator. Owns only this throwaway custom right.
set -euo pipefail
if [[ $EUID -ne 0 ]]; then
    echo 'Run this installer with sudo from your own terminal.' >&2
    exit 1
fi
stage="$(cd "$(dirname "$0")" && pwd)"
base='/Library/Application Support/PortholeAuthorizationPrototype'
bundle='/Library/Security/SecurityAgentPlugins/PortholeAuthorizationPrototype.bundle'
label='work.flotilla.authorization-prototype'
plist="/Library/LaunchDaemons/$label.plist"
runtime='/var/run/porthole-auth-prototype'
right='work.flotilla.prototype.remote-approval'
expected='identifier "work.flotilla.authorization-prototype" and anchor apple generic and certificate leaf[subject.OU] = "973L4GV58R"'
probe="$base/authorization-probe"

[[ ${SUDO_USER:-} == robert ]] || { echo 'This comte experiment expects sudo invoked by robert.' >&2; exit 1; }
[[ $(/usr/bin/id -u robert) == 501 ]] || { echo 'Unexpected robert UID.' >&2; exit 1; }
if /usr/bin/security authorizationdb read "$right" >/dev/null 2>&1; then
    echo "Refusing to replace existing right: $right" >&2; exit 1
fi
for path in "$base" "$bundle" "$plist" "$runtime"; do
    [[ ! -e $path && ! -L $path ]] || { echo "Refusing to replace $path" >&2; exit 1; }
done
# A leading '=' marks inline requirement text; without it codesign expects a file.
/usr/bin/codesign --verify --strict -R "=$expected" "$stage/PortholeAuthorizationPrototype.bundle"
/usr/bin/codesign --verify --strict -R '=anchor apple generic and certificate leaf[subject.OU] = "973L4GV58R"' "$stage/authorization-probe"
/usr/bin/plutil -lint "$stage/right.plist" >/dev/null
[[ $(/usr/libexec/PlistBuddy -c 'Print :class' "$stage/right.plist") == evaluate-mechanisms ]]
[[ $(/usr/libexec/PlistBuddy -c 'Print :mechanisms:0' "$stage/right.plist") == 'PortholeAuthorizationPrototype:approve,privileged' ]]

/bin/mkdir -m 755 "$base" "$runtime"
/usr/bin/install -o root -g wheel -m 755 "$stage/authorization-probe" "$probe"
/usr/bin/install -o root -g wheel -m 644 "$stage/operator-key.pub" "$base/operator-key.pub"
# Keep the original policies as read-only evidence; no system rights are changed.
/usr/bin/security authorizationdb read system.privilege.taskport > "$base/taskport-before.plist"
/usr/bin/ditto "$stage/PortholeAuthorizationPrototype.bundle" "$bundle"
/usr/sbin/chown -R root:wheel "$base" "$runtime" "$bundle"
/bin/chmod -R go-w "$base" "$bundle"

write_job() {
    local extra=''
    [[ $1 != automatic ]] || extra='<string>--automatic</string>'
    /bin/cat > "$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>$label</string>
<key>ProgramArguments</key><array>
<string>$probe</string><string>serve</string><string>$runtime/broker.sock</string>
<string>$base/operator-key.pub</string>$extra
</array>
<key>RunAtLoad</key><true/>
<key>StandardOutPath</key><string>$base/broker.stdout.log</string>
<key>StandardErrorPath</key><string>$base/broker.stderr.log</string>
</dict></plist>
PLIST
    /usr/sbin/chown root:wheel "$plist"
    /bin/chmod 644 "$plist"
    /bin/launchctl bootstrap system "$plist"
    for ((i=0; i<100; i++)); do
        [[ ! -S $runtime/broker.sock ]] || return 0
        /bin/sleep 0.1
    done
    echo 'Broker did not bind; inspect broker.stderr.log.' >&2
    return 1
}

write_job automatic
/usr/bin/security authorizationdb write "$right" < "$stage/right.plist"
echo 'Requesting only the custom test right as robert (automatic policy)...'
# Capture a real AuthorizationCopyRights result, not just a broker response.
set +e
/usr/bin/sudo -u robert "$probe" request > "$base/automatic-result.log" 2>&1
result=$?
set -e
/bin/cat "$base/automatic-result.log"
/bin/launchctl bootout "system/$label"
/bin/rm -f "$runtime/broker.sock"
write_job human
/usr/bin/security authorizationdb read system.privilege.taskport > "$base/taskport-after.plist"
/usr/bin/cmp "$base/taskport-before.plist" "$base/taskport-after.plist"
if [[ $result -ne 0 ]]; then
    echo 'AUTOMATIC TEST FAILED. Human-mode broker is running; preserve logs for diagnosis.' >&2
    exit "$result"
fi
echo 'AUTOMATIC TEST PASSED. Broker now waits for signed decisions from kiwi.'
echo "Installed only $right. Existing taskport policy is unchanged."
