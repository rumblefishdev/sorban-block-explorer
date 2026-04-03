STAGING_APP := node dist/bin/staging.js
PRODUCTION_APP := node dist/bin/production.js
BASTION_STAGING_APP := node dist/bin/bastion-staging.js
BASTION_PRODUCTION_APP := node dist/bin/bastion-production.js

build:
	cd .. && npx nx build @rumblefish/soroban-block-explorer-aws-cdk

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

deploy-staging-partition: build
	npx cdk --app "$(STAGING_APP)" deploy Explorer-staging-Partition --require-approval broadening

deploy-staging-ingestion: build
	npx cdk --app "$(STAGING_APP)" deploy Explorer-staging-Ingestion --require-approval broadening

deploy-staging-bastion: build
	npx cdk --app "$(BASTION_STAGING_APP)" deploy --require-approval broadening

destroy-staging-bastion: build
	npx cdk --app "$(BASTION_STAGING_APP)" destroy --force

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

deploy-production-partition: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Explorer-production-Partition --require-approval broadening

deploy-production-ingestion: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Explorer-production-Ingestion --require-approval broadening

deploy-production-bastion: build
	npx cdk --app "$(BASTION_PRODUCTION_APP)" deploy --require-approval broadening

destroy-production-bastion: build
	npx cdk --app "$(BASTION_PRODUCTION_APP)" destroy --force

# --- Shared ---

bootstrap:
	npx cdk bootstrap
