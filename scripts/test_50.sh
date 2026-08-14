#!/bin/bash
# Test harness — fires N POST /query requests against the live 12377.cn API.
# Captures per-query success/failure and writes raw responses to a JSONL file.
# Adds a 1.5s delay between queries to avoid rate-limiting.

set -u
URL="http://127.0.0.1:8767/query"
CODE="${1:-H026061219520245669B}"
N="${2:-50}"
DELAY="${3:-1.5}"
OUT="/tmp/i12377_responses.jsonl"
LOG="/tmp/i12377_test.log"
: > "$OUT"

if [ ! -f "$LOG" ]; then touch "$LOG"; fi
LOG_OFFSET=$(wc -l < "$LOG")

START=$(date +%s)
for i in $(seq 1 "$N"); do
  RESP=$(curl -sS -X POST "$URL" \
    -H 'Content-Type: application/json' \
    -d "{\"retrieval_code\":\"$CODE\"}" \
    --max-time 60)
  echo "{\"i\":$i,\"resp\":$RESP}" >> "$OUT"
  printf "[%2d/%d] " "$i" "$N"
  echo "$RESP" | head -c 160
  echo ""
  if [ "$i" -lt "$N" ]; then sleep "$DELAY"; fi
done
END=$(date +%s)

echo "========================================"
echo "Wall time: $((END-START))s for $N queries (delay=${DELAY}s)"
echo ""
echo "--- Per-query outcomes ---"
python -c "
import json, sys
n=ok=fail=captcha_err=other=0
total_records=0
with open('$OUT') as f:
    for line in f:
        r=json.loads(line)['resp']
        n+=1
        if r.get('success'):
            ok+=1
            total_records += r.get('total',0)
        else:
            err=r.get('error','') or ''
            if 'captcha' in err.lower(): captcha_err+=1
            else: other+=1
print(f'Total queries  : {n}')
print(f'Successful     : {ok} ({ok*100//max(n,1)}%)')
print(f'Failed (captcha): {captcha_err}')
print(f'Failed (other) : {other}')
print(f'Total records  : {total_records}')
"
echo ""
echo "--- Captcha solver metrics (from server log since test start) ---"
python -c "
captcha_solved=0
captcha_failed=0
captcha_rejected=0
submit_failed=0
with open('$LOG') as f:
    lines = f.readlines()[$LOG_OFFSET:]
    for line in lines:
        if 'captcha solved' in line: captcha_solved += 1
        elif 'captcha recognition failed' in line: captcha_failed += 1
        elif 'server rejected captcha' in line: captcha_rejected += 1
        elif 'submit failed' in line: submit_failed += 1
total_attempts = captcha_solved + captcha_failed + captcha_rejected + submit_failed
recognized = captcha_solved + captcha_rejected
print(f'Captcha fetch+process attempts : {total_attempts}')
print(f'  ddddocr recognized            : {recognized} ({recognized*100//max(total_attempts,1)}%)')
print(f'    → answer accepted (1000)    : {captcha_solved}')
print(f'    → answer rejected (3104)    : {captcha_rejected}')
print(f'  ddddocr failed                : {captcha_failed} ({captcha_failed*100//max(total_attempts,1)}%)')
print(f'  submit step network error     : {submit_failed}')
"