# machine-locks

Server for locking mq.sh and iq.sh. Hamzah's project

## Design

This repository is split into a HTTP server and a CLI client to be called by
both `mq.sh` and `iq.sh`. All server state (machines, lock status) is stored in
a SQLite database, the server program itself is a stateless interface to that
database. Access to machines is governed in a FIFO fashion, where a client
posts a 'job' for a machine, then can poll that job until it is ready, and then
release the job. This also means a single user can push multiple jobs to run in
succession, which was not possible in the old locking interface. Polling a job
is done with HTTP long polling (the server keeps open the connection for some time)
to reduce overhead and allow clients to acquire locks sooner.
