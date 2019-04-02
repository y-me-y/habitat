module "cd_infrastructure" {
  source      = "git@github.com:chef/es-terraform.git//modules/cd_common_infrastructure"
  environment = "delivered"
}

provider "aws" {
  region  = "${module.cd_infrastructure.aws_region}"
  profile = "${module.cd_infrastructure.aws_profile}"
}

module "habitat_sh_site" {
  source    = "git@github.com:chef/es-terraform.git//modules/cd_generic_static_site"
  subdomain = "habitat-sh-${var.dns_suffix}"

  site_dir      = "../../www"
  content_dir   = "build"
  build_command = "BUILDER_WEB_URL='https://bldr.acceptance.habitat.sh' GITHUB_APP_URL='https://github.com/apps/habitat-builder-acceptance' make build"

  fastly_fqdn = "${var.fastly_fqdn}"
  
  # AWS Tags
  tag_dept    = "CoreEng"
  tag_contact = "releng"
}
