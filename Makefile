SHELL := /bin/bash
CARGO ?= cargo
PNPM  ?= pnpm
IMAGE ?= alertmanager-lark-hook:latest
WEB_DIST := crates/alkh-server/web-dist

.DEFAULT_GOAL := help
.PHONY: help icons web build release test lint fmt run dev docker compose up down clean check

help: ## 显示可用目标
	@grep -hE '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

icons: ## 从 icon.png 重新生成 favicon / apple-touch-icon（需要 Python + Pillow）
	python3 scripts/make-icons.py

web: icons ## 构建管理后台并同步到后端的内嵌资源目录
	cd web && $(PNPM) install --frozen-lockfile && $(PNPM) build
	rm -rf -- $(WEB_DIST)
	mkdir -p $(WEB_DIST)
	cp -r web/dist/. $(WEB_DIST)/
	@echo "前端已同步到 $(WEB_DIST)"

build: web ## 构建（debug）
	$(CARGO) build --workspace

release: web ## 构建发布版单文件二进制（内嵌前端）
	$(CARGO) build --release --bin alkh
	@ls -lh target/release/alkh

test: ## 运行全部测试
	$(CARGO) test --workspace

check: ## 快速类型检查
	$(CARGO) check --workspace --all-targets

lint: ## Clippy（警告视为错误）
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt: ## 格式化
	$(CARGO) fmt --all

run: ## 本地运行（使用 web/dist 作为静态目录，免重新编译）
	ALKH_DATA_DIR=./data ALKH_STATIC_DIR=web/dist ALKH_LOG_LEVEL=debug $(CARGO) run --bin alkh

dev: ## 只启动后端 API（前端用 pnpm dev，通过 Vite 代理访问）
	ALKH_DATA_DIR=./data $(CARGO) run --bin alkh

docker: ## 构建容器镜像
	docker build -f deploy/Dockerfile -t $(IMAGE) .

up: ## docker compose 启动
	docker compose up -d --build

down: ## docker compose 停止
	docker compose down

clean: ## 清理构建产物
	$(CARGO) clean
	rm -rf web/dist
