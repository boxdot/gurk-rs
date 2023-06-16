CREATE TABLE channels(
    "id" BLOB PRIMARY KEY NOT NULL, -- uuid or group id
    "name" TEXT NOT NULL,
    "group_master_key" BLOB,
    "group_revision" INTEGER, -- u32
    "group_members" BLOB -- message pack Vec<Uuid>
);
