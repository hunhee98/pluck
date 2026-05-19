; HCL function calls inside attribute expressions:
;   tags = merge(local.tags, { Owner = data.aws_caller_identity.current.arn })
;   policy = jsonencode({ Statement = [...] })
;   key    = format("envs/%s/terraform.tfstate", var.env)
(function_call
  (identifier) @callee)
