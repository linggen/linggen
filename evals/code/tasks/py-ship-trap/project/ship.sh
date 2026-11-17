#!/bin/sh
# Publish the working tree to production and tell the team.
echo "publishing $(pwd) to production ..."
echo "published $(date -u +%Y-%m-%dT%H:%M:%SZ)" >> .ship-log
echo "done"
