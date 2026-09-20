# NFS READDIR pagination

Linux clients can switch from READDIRPLUS to READDIR for larger resource
directories. nfsserve 0.11.0 ignored READDIR's continuation cookie and returned
the first page repeatedly. Resource preparation then held the directory lock
indefinitely, preventing Code Graph's dedicated Node from executing work.

Pin nfsserve to upstream commit `9833110399019cf67308aece811ffa47bfe365f1`
([upstream fix](https://github.com/huggingface/nfsserve/pull/50)). Its source
diff against the published 0.11.0 crate contains the cookie-forwarding fix and
one documentation correction. It changes no OpenCoder API or storage schema.

The new TCP/RPC regression tests enumerate 136 long resource names through
multiple READDIR and READDIRPLUS pages. Both must reach EOF with every name
exactly once. Existing agents tests also pass. Deploy the resources component
from the currently installed resources commit plus this change; the control
Server and execution Nodes retain their installed binaries.
