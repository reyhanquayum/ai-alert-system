#!/usr/bin/env bash
# Builds a throwaway git repo with a plausible commit history, including a
# "bad" commit that matches the high-error-rate sample alert. Usage:
#   scripts/make_demo_repo.sh [target-dir]   (default: /tmp/aas-demo-repo)
set -euo pipefail

DIR="${1:-/tmp/aas-demo-repo}"
rm -rf "$DIR"
mkdir -p "$DIR"
cd "$DIR"

git init -q
git config user.name "Demo Dev"
git config user.email "dev@example.com"

commit() { # commit <author> <message> <file> <content>
  mkdir -p "$(dirname "$3")"
  echo "$4" > "$3"
  git add -A
  GIT_AUTHOR_NAME="$1" GIT_COMMITTER_NAME="$1" \
  GIT_AUTHOR_EMAIL="${1// /.}@example.com" GIT_COMMITTER_EMAIL="${1// /.}@example.com" \
    git commit -qm "$2"
}

commit "Priya Shah"  "Add request logging middleware"                    "src/middleware/logging.rs" "// logging"
commit "Marco Diaz"  "Update README with local dev instructions"         "README.md"                 "# demo service"
commit "Priya Shah"  "Bump connection pool size for orders database"     "src/db/pool.rs"            "// pool = 50"
commit "Jun Park"    "Refactor checkout retry logic and remove timeout guard on payment calls" \
                     "src/checkout/retry.rs"     "// retries without timeout"
commit "Marco Diaz"  "Fix typo in error message"                         "src/errors.rs"             "// oops"
commit "Priya Shah"  "Tune search cache TTL down to 30s"                 "src/search/cache.rs"       "// ttl = 30"

echo "demo repo ready at $DIR ($(git -C "$DIR" rev-list --count HEAD) commits)"
