module "cd_infrastructure" {
  source      = "git@github.com:chef/es-terraform.git//modules/cd_common_infrastructure"
  environment = "delivered"
}

provider "aws" {
  region  = "${module.cd_infrastructure.aws_region}"
  profile = "${module.cd_infrastructure.aws_profile}"
}

# data "external" "latest_hab_pkg" {
#   program = ["bash", "-c", "${path.module}/scripts/latest-hab-pkg.sh"]

#   query = {
#     channel = "${var.channel}"
#   }
# }

resource "null_resource" "build_habitat_sh" {
  triggers = {
    always_do = "${uuid()}"
  }

  provisioner "local-exec" {
    command = "make build"
    working_dir = "../../www"
  }

  provisioner "local-exec" {
    command = "make deploy"
    working_dir = "../../www"
  }

  # provisioner "local-exec" {
  #   command = "chmod +x /tmp/chef-automate"
  # }

  # provisioner "local-exec" {
  #   command = "/tmp/chef-automate airgap bundle create /tmp/automate.aib --channel ${var.channel}"
  # }
}

module "habitat_sh_site" {
  source    = "git@github.com:chef/es-terraform.git//modules/cd_s3_website"
  subdomain = "habitat-sh-${var.dns_suffix}"
  create = "true"
  # create    = "${var.environment == "delivered" ? "true" : "false"}"
}



# module "habitat_sh_site" {
#   source    = "git@github.com:chef/es-terraform.git//modules/cd_hugo_static_site"
#   subdomain = "habitat-sh-${var.dns_suffix}"

#   site_dir     = "./build"
#   fastly_fqdn = "${var.fastly_fqdn}"

#   # build_command = "BUILDER_WEB_URL='https://bldr.acceptance.habitat.sh' GITHUB_APP_URL='https://github.com/apps/habitat-builder-acceptance' make build"

#   build_command = "pwd"

#   # AWS Tags
#   tag_dept    = "CoreEng"
#   tag_contact = "releng"
# }
