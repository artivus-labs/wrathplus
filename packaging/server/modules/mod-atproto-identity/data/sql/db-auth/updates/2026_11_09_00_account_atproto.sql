-- mod-atproto-identity: which ATProto identity (DID, permanent) owns a game account, and its
-- handle as last verified (shown in game). Written only by `.atproto link`, which the WrathPlus
-- account service runs over SOAP after it has proven the identity. One account per DID.
CREATE TABLE IF NOT EXISTS `account_atproto` (
    `account_id` INT UNSIGNED NOT NULL,
    `did` VARCHAR(255) NOT NULL,
    `handle` VARCHAR(253) NOT NULL,
    `linked_at` TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    PRIMARY KEY (`account_id`),
    UNIQUE KEY `uk_did` (`did`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
