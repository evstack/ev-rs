use evolve_core::define_error;

define_error!(ERR_ACCOUNT_DOES_NOT_EXIST, 0x1, "account does not exist");
define_error!(ERR_CODE_NOT_FOUND, 0x2, "account code not found");
define_error!(
    ERR_EXEC_IN_QUERY,
    0x3,
    "exec functionality not available during queries"
);
define_error!(ERR_TOO_MANY_OBJECTS, 0x4, "too many objects");
define_error!(ERR_INVALID_CODE_ID, 0x5, "invalid code identifier");
define_error!(ERR_EVENT_NAME_TOO_LONG, 0x6, "event name too long");
define_error!(ERR_EVENT_CONTENT_TOO_LARGE, 0x7, "event content too large");
define_error!(ERR_SAME_CODE_MIGRATION, 0x8, "cannot migrate to same code");
define_error!(
    ERR_FUNDS_TO_SYSTEM_ACCOUNT,
    0x9,
    "cannot send funds to system accounts"
);
define_error!(
    ERR_OVERLAY_SIZE_EXCEEDED,
    0x0A,
    "overlay size limit exceeded"
);
define_error!(ERR_TOO_MANY_EVENTS, 0x10, "too many events emitted");
define_error!(ERR_KEY_TOO_LARGE, 0x11, "storage key too large");
define_error!(ERR_VALUE_TOO_LARGE, 0x12, "storage value too large");
define_error!(
    ERR_INVALID_CHECKPOINT,
    0x13,
    "checkpoint does not belong to this execution state"
);
