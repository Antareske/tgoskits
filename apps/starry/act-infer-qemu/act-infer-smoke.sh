#!/bin/sh
set -eu

echo "ACT_INFER_BEGIN"
if /usr/bin/act_infer /opt/act; then
    echo "ACT_INFER_OK"
else
    echo "ACT_INFER_FAILED"
    exit 1
fi
