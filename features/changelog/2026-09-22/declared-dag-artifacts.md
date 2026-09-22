# Declared DAG artifact downloads

The worker previously limited DAG downloads to output.txt, output.json and meta.json, so the Native report ZIP and raw evidence were rejected despite valid authenticated links. It now accepts files declared by the step artifact manifest or report archive receipt, validates the declared size and SHA-256, confines reads to the selected step or dynamic instance, and rejects changed files during streaming.

Validation: declared ZIP and nested evidence, undeclared/traversal/symlink rejection, tampered content, and the existing 256 MiB bounded-memory streaming regression all pass. Shared-entry downloads are verified after deployment.
