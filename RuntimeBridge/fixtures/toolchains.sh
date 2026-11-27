#!/bin/sh
# Trusted, fixed development workflow in a disposable guest. No project mounts.
set -eu
for pkg in nodejs npm python3 git; do
    if /sbin/apk info -e "$pkg" >/dev/null 2>&1; then
        echo "Unexpected preinstalled toolchain: $pkg" >&2
        exit 32
    fi
done
printf 'NEXUS_TOOLCHAINS_ABSENT\n'
/sbin/apk --repositories-file /dev/null \
    --repository https://dl-cdn.alpinelinux.org/alpine/v3.21/main \
    --repository https://dl-cdn.alpinelinux.org/alpine/v3.21/community \
    --no-cache --no-scripts add nodejs npm python3 git >&2
installed=$(/sbin/apk --no-network info -v)
for pkg in nodejs npm python3 git; do
    /sbin/apk info -e "$pkg" >/dev/null
    version=$(printf '%s\n' "$installed" | grep -E "^$pkg-[0-9][^[:space:]]*-r[0-9]+$")
    printf 'NEXUS_TOOLCHAIN_APK=%s\n' "$version"
done
# Guest V8 machine code is not host JIT. For this fixture disable V8 JIT too.
export NODE_OPTIONS=--jitless
printf 'NEXUS_NODE_VERSION=%s\n' "$(node --version)"
printf 'NEXUS_NPM_VERSION=%s\n' "$(npm --version)"
printf 'NEXUS_PYTHON_VERSION=%s\n' "$(python3 --version)"
printf 'NEXUS_GIT_VERSION=%s\n' "$(git --version)"
work=$(mktemp -d /tmp/nexus-toolchains.XXXXXX)
cd "$work"
export HOME="$work/home"
mkdir -p "$HOME" dep
export npm_config_cache="$work/npm-cache"
export npm_config_userconfig="$work/npmrc"
export GIT_CONFIG_NOSYSTEM=1
export GIT_CONFIG_GLOBAL=/dev/null
export PYTHONDONTWRITEBYTECODE=1
# No registry fetch or lifecycle hooks: install a known local fixture.
cat > dep/package.json <<'EOF'
{"name":"nexus-local-fixture","version":"1.0.0","main":"index.js"}
EOF
cat > dep/index.js <<'EOF'
module.exports = (a,b) => a+b;
EOF
cat > package.json <<'EOF'
{"name":"nexus-probe","version":"1.0.0","private":true,"dependencies":{"nexus-local-fixture":"file:./dep"}}
EOF
npm install --offline --ignore-scripts --no-audit --no-fund --package-lock >&2
test -f package-lock.json
node -e 'if (require("nexus-local-fixture")(20,22) !== 42) process.exit(1); console.log("NEXUS_NODE_DEPENDENCY_OK")'
cat > test_math.py <<'EOF'
import unittest

class MathTest(unittest.TestCase):
    def test_sum(self):
        self.assertEqual(sum([20, 22]), 42)

if __name__ == "__main__":
    unittest.main()
EOF
python3 test_math.py >&2
printf 'NEXUS_PYTHON_TEST_OK\n'
git init -q
git -c core.hooksPath=/dev/null add package.json package-lock.json dep test_math.py
git -c core.hooksPath=/dev/null -c user.name='Nexus Fixture' -c user.email='fixture@localhost' -c commit.gpgsign=false commit -qm 'fixture'
git diff --exit-code HEAD -- package.json package-lock.json dep test_math.py
hash=$(git rev-parse --verify HEAD)
printf 'NEXUS_GIT_COMMIT=%s\n' "$hash"
printf 'NEXUS_TOOLCHAINS_OK\n'
