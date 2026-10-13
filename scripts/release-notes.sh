#!/usr/bin/env bash
#
# 生成 GitHub Release 的发布说明（release body）。
#
# 内容分三块：
#   1. 容器镜像：拉取命令、digest、平台 —— 让镜像信息直接出现在 Release 页面；
#   2. 变更内容：按 Conventional Commits 把上一个 tag 到本次 tag 之间的提交分组；
#   3. 发布产物：Release 附件（镜像离线包、校验和）的大小与 SHA-256。
#
# 本脚本只读 git 历史与传入的文件，不访问网络，可以在本地随意试跑：
#
#   scripts/release-notes.sh --tag 0.1.0 --image ghcr.io/fengyifan/alertmanager-lark-hook \
#       --digest sha256:deadbeef --platforms linux/amd64 --asset dist/foo.tar.gz
#
set -euo pipefail

TAG=""
FROM=""
REPO="${GITHUB_REPOSITORY:-}"
IMAGE=""
DIGEST=""
PLATFORMS=""
OUT=""
ASSETS=()

die() { printf 'release-notes: %s\n' "$*" >&2; exit 1; }

usage() {
    cat <<'EOF'
用法: release-notes.sh --tag <X.Y.Z> [选项]

  --tag <tag>          必需。要发布的 tag，例如 0.1.0（不带 v 前缀）
  --from <tag>         起始 tag（不含）；默认用 git describe 找 tag 的前一个可达 tag
  --repo <owner/repo>  仓库，用于生成提交/比较链接；默认取 $GITHUB_REPOSITORY 或 origin
  --image <ref>        镜像名（不含 tag），例如 ghcr.io/fengyifan/alertmanager-lark-hook
  --digest <sha256:…>  镜像 digest
  --platforms <列表>   镜像平台，例如 linux/amd64
  --asset <文件>       发布产物文件，可重复；会写入大小与 SHA-256
  --out <文件>         输出到文件；默认 stdout
  -h, --help           显示本帮助
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --tag)       TAG="${2:?--tag 缺少取值}"; shift 2 ;;
        --from)      FROM="${2:?--from 缺少取值}"; shift 2 ;;
        --repo)      REPO="${2:?--repo 缺少取值}"; shift 2 ;;
        --image)     IMAGE="${2:?--image 缺少取值}"; shift 2 ;;
        --digest)    DIGEST="${2:?--digest 缺少取值}"; shift 2 ;;
        --platforms) PLATFORMS="${2:?--platforms 缺少取值}"; shift 2 ;;
        --asset)     ASSETS+=("${2:?--asset 缺少取值}"); shift 2 ;;
        --out)       OUT="${2:?--out 缺少取值}"; shift 2 ;;
        -h|--help)   usage; exit 0 ;;
        *)           usage >&2; die "未知参数：$1" ;;
    esac
done

[ -n "$TAG" ] || { usage >&2; die "缺少 --tag"; }
[[ "$TAG" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]] || die "tag 必须是 X.Y.Z 形式（不带 v 前缀），实际为：$TAG"
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || die "本地不存在 tag $TAG"

if [ -z "$REPO" ]; then
    origin=$(git remote get-url origin 2>/dev/null || true)
    case "$origin" in
        *github.com[:/]*) REPO="${origin#*github.com[:/]}"; REPO="${REPO%.git}" ;;
        *) REPO="" ;;
    esac
fi
[ -n "$REPO" ] || die "无法确定仓库名，请显式传 --repo owner/repo"
REPO_URL="https://github.com/$REPO"
VERSION="$TAG"

# 起始 tag：显式传入 > 自动查找（tag 的前一个可达 tag）。
# --match '[0-9]*' 只认数字开头的版本 tag，避免把无关 tag 当成上一个版本。
if [ -z "$FROM" ]; then
    FROM=$(git describe --tags --abbrev=0 --match '[0-9]*' "${TAG}^" 2>/dev/null || true)
fi
if [ -n "$FROM" ]; then
    git rev-parse -q --verify "refs/tags/$FROM" >/dev/null || die "本地不存在起始 tag $FROM"
    RANGE="$FROM..$TAG"
else
    RANGE="$TAG"
fi

TMPDIR_NOTES=$(mktemp -d)
trap 'rm -rf "$TMPDIR_NOTES"' EXIT

# 提交按类型分流到不同文件；文件名即输出顺序
declare -A SECTIONS=(
    [breaking]='### ⚠️ 破坏性变更'
    [feat]='### ✨ 新功能'
    [fix]='### 🐛 问题修复'
    [perf]='### ⚡ 性能优化'
    [refactor]='### ♻️ 重构'
    [docs]='### 📝 文档'
    [test]='### ✅ 测试'
    [build]='### 🔧 构建与 CI'
    [other]='### 🧹 其他改动'
)
for key in breaking feat fix perf refactor docs test build other; do
    : > "$TMPDIR_NOTES/$key"
done

total=0
# %x1e 作记录分隔符，%x1f 作字段分隔符，避免正文里的换行干扰解析
while IFS= read -r -d $'\x1e' record; do
    [ -n "${record//[$'\n\t ']/}" ] || continue
    sha="${record%%$'\x1f'*}"
    rest="${record#*$'\x1f'}"
    subject="${rest%%$'\x1f'*}"
    body="${rest#*$'\x1f'}"
    subject="${subject//$'\n'/}"
    sha="${sha//$'\n'/}"
    [ -n "$sha" ] || continue
    total=$((total + 1))

    # Conventional Commits: type(scope)!: description
    section="other"
    line=""
    if [[ "$subject" =~ ^([a-zA-Z]+)(\(([^()]*)\))?(!)?:[[:space:]]+(.*)$ ]]; then
        ctype=$(printf '%s' "${BASH_REMATCH[1]}" | tr '[:upper:]' '[:lower:]')
        cscope="${BASH_REMATCH[3]}"
        cbang="${BASH_REMATCH[4]}"
        cdesc="${BASH_REMATCH[5]}"
        case "$ctype" in
            feat)      section=feat ;;
            fix)       section=fix ;;
            perf)      section=perf ;;
            refactor)  section=refactor ;;
            docs)      section=docs ;;
            test)      section=test ;;
            build|ci)  section=build ;;
            *)         section=other ;;
        esac
        if [ -n "$cscope" ]; then
            line="**$cscope**: $cdesc"
        else
            line="$cdesc"
        fi
        if [ -n "$cbang" ] || printf '%s' "$body" | grep -q 'BREAKING CHANGE'; then
            section=breaking
        fi
    else
        line="$subject"
    fi

    printf -- '- %s（[%s](%s/commit/%s)）\n' \
        "$line" "${sha:0:7}" "$REPO_URL" "$sha" >> "$TMPDIR_NOTES/$section"
done < <(git log --no-merges --format='%H%x1f%s%x1f%b%x1e' "$RANGE")

emit() { printf '%s\n' "$*"; }

# ---------------------------------------------------------------------------
# 输出
# ---------------------------------------------------------------------------
{
    if [ -n "$IMAGE" ]; then
        emit '## 📦 容器镜像'
        emit
        emit '| 项目 | 值 |'
        emit '|---|---|'
        emit "| 镜像 | \`$IMAGE:$VERSION\` |"
        if [ -n "$DIGEST" ]; then emit "| Digest | \`$DIGEST\` |"; fi
        if [ -n "$PLATFORMS" ]; then emit "| 平台 | \`$PLATFORMS\` |"; fi
        emit
        emit '```bash'
        emit "docker pull $IMAGE:$VERSION"
        emit '```'
        emit
        emit "> 也可直接使用下方附件的离线镜像包：\`docker load -i <文件名>\`，加载后镜像名为 \`$IMAGE:$VERSION\`。"
        emit
    fi

    emit '## 📝 变更内容'
    emit
    if [ "$total" -eq 0 ]; then
        emit '_本次 tag 与上一个 tag 之间没有独立提交（例如仅合并提交）。_'
        emit
    else
        for key in breaking feat fix perf refactor docs test build other; do
            if [ -s "$TMPDIR_NOTES/$key" ]; then
                emit "${SECTIONS[$key]}"
                emit
                cat "$TMPDIR_NOTES/$key"
                emit
            fi
        done
    fi

    if [ "${#ASSETS[@]}" -gt 0 ]; then
        emit '## 📎 发布产物'
        emit
        emit '| 文件 | 大小 | SHA-256 |'
        emit '|---|---|---|'
        for asset in "${ASSETS[@]}"; do
            name=$(basename "$asset")
            bytes=$(wc -c < "$asset")
            if command -v numfmt >/dev/null 2>&1; then
                size=$(numfmt --to=iec-i --suffix=B "$bytes")
            else
                size="${bytes}B"
            fi
            hash=$(sha256sum "$asset" | cut -d' ' -f1)
            emit "| \`$name\` | $size | \`$hash\` |"
        done
        emit
        emit "校验和汇总见附件 \`sha256sums.txt\`。"
        emit
    fi

    if [ -n "$FROM" ]; then
        emit "**完整变更**：[\`$FROM...$TAG\`]($REPO_URL/compare/$FROM...$TAG)"
    else
        emit "**本版本提交**：[\`$TAG\`]($REPO_URL/tree/$TAG)（首个版本，无上一个 tag 可比较）"
    fi
} > "${OUT:-/dev/stdout}"
