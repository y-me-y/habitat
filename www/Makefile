build: clean
	bundle install
	bundle exec middleman build --no-parallel
.PHONY: build

clean:
	rm -rf build

run: build
	BUILDER_WEB_URL="http://localhost:3000" GITHUB_APP_URL="https://github.com/apps/habitat-builder" bundle exec middleman serve

sync: build check-env
	bundle exec middleman s3_sync
.PHONY: sync

purge_cache: check-env
	curl -H "Fastly-Key: ${FASTLY_API_KEY}" -X POST "https://api.fastly.com/service/${FASTLY_SERVICE_KEY}/purge_all"

deploy: build sync purge_cache

preprod: build
ifndef CHANNEL
	$(error CHANNEL is undefined)
endif
	sed -i'' '/^Disallow:/ s/$$/ \//' build/robots.txt
	cd ../terraform/habitat-sh-deploy && make plan

cli_docs:
	hab studio run "hab pkg install core/hab-studio && \
	hab pkg install core/hab-sup && \
	hab pkg install core/hab-launcher && \
	hab pkg install core/node --binlink && \
	node scripts/generate-cli-docs > source/docs/habitat-cli.html.md"

template_reference:
	cp ../components/sup/doc/render_context_schema.json scripts/schema.json.tmp
	hab studio run "hab pkg install core/node --binlink && \
	cat scripts/schema.json.tmp | node scripts/generate-template-reference.js > ./source/partials/docs/_reference-template-data.html.md.erb"
	rm -f scripts/schema.json.tmp

check-env:
ifndef AWS_ACCESS_KEY_ID
	$(error AWS_ACCESS_KEY_ID is undefined)
endif
ifndef AWS_BUCKET
	$(error AWS_BUCKET is undefined)
endif
ifndef AWS_DEFAULT_REGION
	$(error AWS_DEFAULT_REGION is undefined)
endif
ifndef AWS_SECRET_ACCESS_KEY
	$(error AWS_SECRET_ACCESS_KEY is undefined)
endif
ifndef FASTLY_API_KEY
	$(error FASTLY_API_KEY is undefined)
endif
ifndef FASTLY_SERVICE_KEY
	$(error FASTLY_SERVICE_KEY is undefined)
endif
ifndef NETLIFYKEY
	$(error NETLIFYKEY is undefined)
endif
