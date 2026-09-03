terraform {
  required_version = ">= 1.0.0"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
  }
  backend "s3" {
    bucket         = "my-terraform-state-bucket"
    key            = "forgecode/terraform.tfstate"
    region         = "us-east-1"
    dynamodb_table = "my-terraform-lock-table"
    encrypt        = true
  }
}

provider "aws" {
  alias  = "us_east_1"
  region = "us-east-1"
}

provider "aws" {
  alias  = "eu_west_1"
  region = "eu-west-1"
}

provider "aws" {
  alias  = "ap_southeast_1"
  region = "ap-southeast-1"
}

module "us_east_1_stack" {
  source = "./modules/regional-stack"
  providers = {
    aws = aws.us_east_1
  }
  environment = var.environment
  region      = "us-east-1"
  vpc_cidr    = "10.0.0.0/16"
}

module "eu_west_1_stack" {
  source = "./modules/regional-stack"
  providers = {
    aws = aws.eu_west_1
  }
  environment = var.environment
  region      = "eu-west-1"
  vpc_cidr    = "10.1.0.0/16"
}

module "ap_southeast_1_stack" {
  source = "./modules/regional-stack"
  providers = {
    aws = aws.ap_southeast_1
  }
  environment = var.environment
  region      = "ap-southeast-1"
  vpc_cidr    = "10.2.0.0/16"
}
