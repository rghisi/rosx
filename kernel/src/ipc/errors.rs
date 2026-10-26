use system::error::ErrorCode;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum IpcConnectionError {
    ServerNotFound,
    ConnectionCannotBeEstablished,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum IpcBindingError {
    AlreadyBound,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum IpcSendError {
    ConnectionNotFound,
    ConnectionCongested,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum IpcReceiveError {
    ConnectionNotFound,
    NoMessagesAvailable,
    MailboxNotAvailable,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum DataError {
    ConnectionNotFound,
    TooLarge,
    BudgetExceeded,
    NotFound,
    NotReadable,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum DeliverError {
    ServiceNotFound,
    Congested,
}

impl From<IpcConnectionError> for ErrorCode {
    fn from(error: IpcConnectionError) -> Self {
        match error {
            IpcConnectionError::ServerNotFound => ErrorCode::IpcServerNotFound,
            IpcConnectionError::ConnectionCannotBeEstablished => {
                ErrorCode::IpcConnectionCannotBeEstablished
            }
        }
    }
}

impl From<IpcBindingError> for ErrorCode {
    fn from(error: IpcBindingError) -> Self {
        match error {
            IpcBindingError::AlreadyBound => ErrorCode::IpcAlreadyBound,
        }
    }
}

impl From<IpcSendError> for ErrorCode {
    fn from(error: IpcSendError) -> Self {
        match error {
            IpcSendError::ConnectionNotFound => ErrorCode::IpcSendConnectionNotFound,
            IpcSendError::ConnectionCongested => ErrorCode::IpcSendConnectionCongested,
        }
    }
}

impl From<IpcReceiveError> for ErrorCode {
    fn from(error: IpcReceiveError) -> Self {
        match error {
            IpcReceiveError::ConnectionNotFound => ErrorCode::IpcReceiveConnectionNotFound,
            IpcReceiveError::NoMessagesAvailable => ErrorCode::IpcReceiveNoMessagesAvailable,
            IpcReceiveError::MailboxNotAvailable => ErrorCode::IpcMailboxNotAvailable,
        }
    }
}

impl From<DataError> for ErrorCode {
    fn from(error: DataError) -> Self {
        match error {
            DataError::ConnectionNotFound => ErrorCode::IpcSendConnectionNotFound,
            DataError::TooLarge => ErrorCode::IpcDataTooLarge,
            DataError::BudgetExceeded => ErrorCode::IpcDataBudgetExceeded,
            DataError::NotFound => ErrorCode::IpcDataNotFound,
            DataError::NotReadable => ErrorCode::IpcDataNotReadable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use system::error::ERROR_BIT;

    #[test]
    fn connection_error_variants_map_to_reserved_codes_and_decode_as_errors() {
        for error in [
            IpcConnectionError::ServerNotFound,
            IpcConnectionError::ConnectionCannotBeEstablished,
        ] {
            let raw = ErrorCode::from(error).to_reg();
            assert_ne!(raw & ERROR_BIT, 0);
            assert_eq!(
                ErrorCode::from(error),
                match error {
                    IpcConnectionError::ServerNotFound => ErrorCode::IpcServerNotFound,
                    IpcConnectionError::ConnectionCannotBeEstablished => {
                        ErrorCode::IpcConnectionCannotBeEstablished
                    }
                }
            );
            assert_eq!(ErrorCode::from_code(raw), ErrorCode::from(error));
        }
    }

    #[test]
    fn binding_error_variants_map_to_reserved_codes_and_decode_as_errors() {
        for error in [IpcBindingError::AlreadyBound] {
            let raw = ErrorCode::from(error).to_reg();
            assert_ne!(raw & ERROR_BIT, 0);
            assert_eq!(ErrorCode::from(error), ErrorCode::IpcAlreadyBound);
            assert_eq!(ErrorCode::from_code(raw), ErrorCode::from(error));
        }
    }

    #[test]
    fn send_error_variants_map_to_reserved_codes_and_decode_as_errors() {
        for error in [
            IpcSendError::ConnectionNotFound,
            IpcSendError::ConnectionCongested,
        ] {
            let raw = ErrorCode::from(error).to_reg();
            assert_ne!(raw & ERROR_BIT, 0);
            assert_eq!(
                ErrorCode::from(error),
                match error {
                    IpcSendError::ConnectionNotFound => ErrorCode::IpcSendConnectionNotFound,
                    IpcSendError::ConnectionCongested => ErrorCode::IpcSendConnectionCongested,
                }
            );
            assert_eq!(ErrorCode::from_code(raw), ErrorCode::from(error));
        }
    }

    #[test]
    fn receive_error_variants_map_to_reserved_codes_and_decode_as_errors() {
        for error in [
            IpcReceiveError::ConnectionNotFound,
            IpcReceiveError::NoMessagesAvailable,
            IpcReceiveError::MailboxNotAvailable,
        ] {
            let raw = ErrorCode::from(error).to_reg();
            assert_ne!(raw & ERROR_BIT, 0);
            assert_eq!(
                ErrorCode::from(error),
                match error {
                    IpcReceiveError::ConnectionNotFound => ErrorCode::IpcReceiveConnectionNotFound,
                    IpcReceiveError::NoMessagesAvailable =>
                        ErrorCode::IpcReceiveNoMessagesAvailable,
                    IpcReceiveError::MailboxNotAvailable => ErrorCode::IpcMailboxNotAvailable,
                }
            );
            assert_eq!(ErrorCode::from_code(raw), ErrorCode::from(error));
        }
    }

    #[test]
    fn data_error_variants_map_to_reserved_codes_and_decode_as_errors() {
        for error in [
            DataError::ConnectionNotFound,
            DataError::TooLarge,
            DataError::BudgetExceeded,
            DataError::NotFound,
            DataError::NotReadable,
        ] {
            let raw = ErrorCode::from(error).to_reg();
            assert_ne!(raw & ERROR_BIT, 0);
            assert_eq!(
                ErrorCode::from(error),
                match error {
                    DataError::ConnectionNotFound => ErrorCode::IpcSendConnectionNotFound,
                    DataError::TooLarge => ErrorCode::IpcDataTooLarge,
                    DataError::BudgetExceeded => ErrorCode::IpcDataBudgetExceeded,
                    DataError::NotFound => ErrorCode::IpcDataNotFound,
                    DataError::NotReadable => ErrorCode::IpcDataNotReadable,
                }
            );
            assert_eq!(ErrorCode::from_code(raw), ErrorCode::from(error));
        }
    }
}
