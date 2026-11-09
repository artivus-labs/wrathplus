-- mod-atproto-identity: progress of characters on linked accounts, queued until the account
-- service has published it (`.atproto events` hands rows out, `.atproto ack` deletes them).
-- Rows are inserted asynchronously; the world thread never waits on this table.
CREATE TABLE IF NOT EXISTS `atproto_event` (
    `id` BIGINT UNSIGNED NOT NULL,
    `occurred_at` INT UNSIGNED NOT NULL,
    `did` VARCHAR(255) NOT NULL,
    `guid` INT UNSIGNED NOT NULL,
    `kind` VARCHAR(16) NOT NULL,
    `value` INT UNSIGNED NOT NULL,
    `level` TINYINT UNSIGNED NOT NULL,
    `race` TINYINT UNSIGNED NOT NULL,
    `class` TINYINT UNSIGNED NOT NULL,
    `gender` TINYINT UNSIGNED NOT NULL,
    `name` VARCHAR(48) NOT NULL,
    `title` VARCHAR(512) NOT NULL DEFAULT '',
    PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
