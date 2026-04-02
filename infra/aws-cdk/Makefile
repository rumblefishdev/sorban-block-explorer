STAGING_APP := node dist/bin/staging.js
PRODUCTION_APP := node dist/bin/production.js

build:
	cd ../.. && npx nx build @rumblefish/soroban-block-explorer-aws-cdk

# --- Staging ---

synth-staging: build
	npx cdk --app "$(STAGING_APP)" synth

diff-staging: build
	npx cdk --app "$(STAGING_APP)" diff

deploy-staging: build
	npx cdk --app "$(STAGING_APP)" deploy --all --require-approval broadening

deploy-staging-network: build
	npx cdk --app "$(STAGING_APP)" deploy Explorer-staging-Network --require-approval broadening

deploy-staging-rds: build
	npx cdk --app "$(STAGING_APP)" deploy Explorer-staging-Rds --require-approval broadening

deploy-staging-ledger-bucket: build
	npx cdk --app "$(STAGING_APP)" deploy Explorer-staging-LedgerBucket --require-approval broadening

deploy-staging-migration: build
	npx cdk --app "$(STAGING_APP)" deploy Explorer-staging-Migration --require-approval broadening

# --- Production ---

synth-production: build
	npx cdk --app "$(PRODUCTION_APP)" synth

diff-production: build
	npx cdk --app "$(PRODUCTION_APP)" diff

deploy-production: build
	npx cdk --app "$(PRODUCTION_APP)" deploy --all --require-approval broadening

deploy-production-network: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Explorer-production-Network --require-approval broadening

deploy-production-rds: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Explorer-production-Rds --require-approval broadening

deploy-production-ledger-bucket: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Explorer-production-LedgerBucket --require-approval broadening

deploy-production-migration: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Explorer-production-Migration --require-approval broadening

# --- Shared ---

bootstrap:
	npx cdk bootstrap
