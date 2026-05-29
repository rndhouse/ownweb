SHARED_FILES = background.js content.css contentScript.js options.css options.html options.js siteAdapters.js

.PHONY: all chrome firefox clean

all: chrome firefox

chrome:
	rm -rf dist/chrome
	mkdir -p dist/chrome
	cp chrome/manifest.json dist/chrome/manifest.json
	for file in $(SHARED_FILES); do cp "shared/$$file" "dist/chrome/$$file"; done

firefox:
	rm -rf dist/firefox
	mkdir -p dist/firefox
	cp firefox/manifest.json dist/firefox/manifest.json
	for file in $(SHARED_FILES); do cp "shared/$$file" "dist/firefox/$$file"; done

clean:
	rm -rf dist
