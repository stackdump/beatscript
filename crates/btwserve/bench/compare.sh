#!/bin/bash
# Run every case in cases.txt (METHOD|path|optional header) against the Go server (GO, default 127.0.0.1:18201)
# and btwserve (RS, default 127.0.0.1:18202); compare status, headers (Date ignored; Content-Length too for
# opus) and body hash. Usage: compare.sh cases.txt   (run in a scratch directory: it writes c<n>.* files)
GO=${GO:-127.0.0.1:18201}; RS=${RS:-127.0.0.1:18202}
n=0; same=0
while IFS='|' read -r m u h; do
  n=$((n+1)); rm -f c$n.*
  for p in "$GO" "$RS"; do
    o=c$n.${p##*:}
    if [ "$m" = HEAD ]; then curl -s -I ${h:+-H "$h"} "http://$p$u" > $o; : > $o.body
    else curl -s -X "$m" -D - ${h:+-H "$h"} "http://$p$u" -o $o.body > $o; touch $o.body; fi
  done
  ign='^(date)'; case "$u" in *opus*) ign='^(date|content-length)';; esac
  a=c$n.${GO##*:}; b=c$n.${RS##*:}
  if diff <(grep -v -i -E "$ign" $a; sha256sum < $a.body) <(grep -v -i -E "$ign" $b; sha256sum < $b.body) > d$n; then
    same=$((same+1)); echo "$n SAME $m $(head -1 $a | tr -d '\r') $u $h"
  else echo "$n DIFF $m $u $h"; head -12 d$n; fi
done < "${1:-cases.txt}"
echo "$same/$n identical"
