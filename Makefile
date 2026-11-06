# BoundlessDeck — common tasks. `make` lists them.
#
# The app lives in app/ (its mygo.json and resources too); self-contained
# packages in internal/. Builds land in build/<os>-<arch>/.

APP     := ./app
MYGO    := go tool mygo
# The Windows installer needs NSIS (makensis); without it, the build skips it.
WIN_INSTALLER := $(if $(shell command -v makensis 2>/dev/null),,-skip-installer)

.DEFAULT_GOAL := help
.PHONY: help dev build build-mac build-windows build-windows-arm build-all \
        test race test-nocgo vet fmt fmt-check check snapshot clean

help: ## List the tasks
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z_-]+:.*## / {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

dev: ## Run in dev mode; restarts on changes
	$(MYGO) dev $(APP)

build: build-mac ## Package the app for this Mac

build-mac: ## Package for macOS (Apple silicon): .app and .dmg
	$(MYGO) build -platform darwin/arm64 $(APP)

build-windows: ## Package for Windows x64 (installer if NSIS is installed)
	$(MYGO) build -platform windows/amd64 $(WIN_INSTALLER) $(APP)

build-windows-arm: ## Package for Windows ARM64
	$(MYGO) build -platform windows/arm64 $(WIN_INSTALLER) $(APP)

build-all: ## Package for macOS and Windows x64
	$(MYGO) build -platform darwin/arm64,windows/amd64 $(WIN_INSTALLER) $(APP)

test: ## Run every test, headless
	go test ./...

race: ## Run the tests with the race detector
	go test -race ./...

test-nocgo: ## Run the tests without cgo (as the shipped app builds)
	CGO_ENABLED=0 go test ./...

vet: ## go vet, and MyGo's lifecycle checks of the native UI
	go vet ./...
	$(MYGO) vet $(APP)

fmt: ## Format the Go code
	gofmt -w app internal

fmt-check: ## Fail when some Go code is not formatted
	@out=$$(gofmt -l app internal); if [ -n "$$out" ]; then echo "not formatted:"; echo "$$out"; exit 1; fi

check: fmt-check vet test ## Everything to run before a commit

snapshot: ## Re-render docs/canvas-snapshot.png and the home screen's
	DECK_SNAPSHOT=1 go test -run 'TestSnapshot|TestHomeSnapshot' $(APP)

clean: ## Remove MyGo's builds (build/<os>-<arch>/) and dev builds; other files in build/ stay
	rm -rf build/darwin-* build/windows-* build/linux-* build/web app/.mygo .mygo app/app boundlessdeck
