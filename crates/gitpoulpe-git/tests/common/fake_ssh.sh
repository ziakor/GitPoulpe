#!/bin/sh
echo "$@" >> "$FAKESSH_DIR/log"
mode=$(cat "$FAKESSH_DIR/mode")
case "$mode" in
  unknown) echo "Host key verification failed." >&2; exit 255 ;;
  changed)
    printf '@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\nHost key verification failed.\n' >&2; exit 255 ;;
  denied) echo "git@fakehost: Permission denied (publickey)." >&2; exit 255 ;;
  offline) echo "ssh: Could not resolve hostname fakehost: Name or service not known" >&2; exit 255 ;;
  hang) sleep 60 & echo $! > "$FAKESSH_DIR/sleeper"; wait ;;
  ok)
    for a; do cmd="$a"; done
    case "$cmd" in
      git-upload-pack*) cmd="git upload-pack${cmd#git-upload-pack}" ;;
      git-receive-pack*) cmd="git receive-pack${cmd#git-receive-pack}" ;;
    esac
    exec sh -c "$cmd" ;;
esac
