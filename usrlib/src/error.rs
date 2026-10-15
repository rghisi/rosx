use core::fmt::{Display, Formatter};
use system::error::ErrorCode;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum IpcConnectionError {
    ServerNotFound,
    ConnectionCannotBeEstablished,
}

impl From<ErrorCode> for IpcConnectionError {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::IpcConnectionCannotBeEstablished => Self::ConnectionCannotBeEstablished,
            _ => Self::ServerNotFound,
        }
    }
}

impl Display for IpcConnectionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            IpcConnectionError::ServerNotFound => write!(f, "Server not found"),
            IpcConnectionError::ConnectionCannotBeEstablished => {
                write!(f, "Connection cannot be established")
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum IpcBindingError {
    AlreadyBound,
}

impl From<ErrorCode> for IpcBindingError {
    fn from(_code: ErrorCode) -> Self {
        Self::AlreadyBound
    }
}

impl Display for IpcBindingError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            IpcBindingError::AlreadyBound => write!(f, "Service already bound"),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum IpcSendError {
    ConnectionNotFound,
    ConnectionCongested,
}

impl From<ErrorCode> for IpcSendError {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::IpcSendConnectionCongested => Self::ConnectionCongested,
            _ => Self::ConnectionNotFound,
        }
    }
}

impl Display for IpcSendError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            IpcSendError::ConnectionNotFound => write!(f, "Connection not found"),
            IpcSendError::ConnectionCongested => write!(f, "Connection congested"),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum IpcReceiveError {
    ConnectionNotFound,
    NoMessagesAvailable,
    MailboxNotAvailable,
}

impl From<ErrorCode> for IpcReceiveError {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::IpcReceiveNoMessagesAvailable => Self::NoMessagesAvailable,
            ErrorCode::IpcMailboxNotAvailable => Self::MailboxNotAvailable,
            _ => Self::ConnectionNotFound,
        }
    }
}

impl Display for IpcReceiveError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            IpcReceiveError::ConnectionNotFound => write!(f, "Connection not found"),
            IpcReceiveError::NoMessagesAvailable => write!(f, "No messages available"),
            IpcReceiveError::MailboxNotAvailable => write!(f, "Mailbox not available"),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DataError {
    TooLarge,
    BudgetExceeded,
    NotFound,
    NotReadable,
}

impl From<ErrorCode> for DataError {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::IpcDataTooLarge => Self::TooLarge,
            ErrorCode::IpcDataBudgetExceeded => Self::BudgetExceeded,
            ErrorCode::IpcDataNotReadable => Self::NotReadable,
            _ => Self::NotFound,
        }
    }
}

impl Display for DataError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            DataError::TooLarge => write!(f, "Data object too large"),
            DataError::BudgetExceeded => write!(f, "Data budget exceeded"),
            DataError::NotFound => write!(f, "Data object not found"),
            DataError::NotReadable => write!(f, "Data object not readable"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_ERROR_CODES: [ErrorCode; 13] = [
        ErrorCode::IpcServerNotFound,
        ErrorCode::IpcConnectionCannotBeEstablished,
        ErrorCode::IpcAlreadyBound,
        ErrorCode::IpcSendConnectionNotFound,
        ErrorCode::IpcSendConnectionCongested,
        ErrorCode::IpcReceiveConnectionNotFound,
        ErrorCode::IpcReceiveNoMessagesAvailable,
        ErrorCode::IpcMailboxNotAvailable,
        ErrorCode::IpcDataTooLarge,
        ErrorCode::IpcDataBudgetExceeded,
        ErrorCode::IpcDataNotFound,
        ErrorCode::IpcDataNotReadable,
        ErrorCode::Unknown,
    ];

    #[test]
    fn every_error_code_maps_through_every_mirror_totally() {
        for code in ALL_ERROR_CODES {
            let connection = IpcConnectionError::from(code);
            let binding = IpcBindingError::from(code);
            let send = IpcSendError::from(code);
            let receive = IpcReceiveError::from(code);
            let data = DataError::from(code);

            let expected_connection = match code {
                ErrorCode::IpcConnectionCannotBeEstablished => {
                    IpcConnectionError::ConnectionCannotBeEstablished
                }
                _ => IpcConnectionError::ServerNotFound,
            };
            let expected_send = match code {
                ErrorCode::IpcSendConnectionCongested => IpcSendError::ConnectionCongested,
                _ => IpcSendError::ConnectionNotFound,
            };
            let expected_receive = match code {
                ErrorCode::IpcReceiveNoMessagesAvailable => IpcReceiveError::NoMessagesAvailable,
                ErrorCode::IpcMailboxNotAvailable => IpcReceiveError::MailboxNotAvailable,
                _ => IpcReceiveError::ConnectionNotFound,
            };
            let expected_data = match code {
                ErrorCode::IpcDataTooLarge => DataError::TooLarge,
                ErrorCode::IpcDataBudgetExceeded => DataError::BudgetExceeded,
                ErrorCode::IpcDataNotReadable => DataError::NotReadable,
                _ => DataError::NotFound,
            };

            assert_eq!(connection, expected_connection);
            assert_eq!(binding, IpcBindingError::AlreadyBound);
            assert_eq!(send, expected_send);
            assert_eq!(receive, expected_receive);
            assert_eq!(data, expected_data);
        }
    }

    #[test]
    fn unknown_code_falls_back_to_catch_all_variant_of_every_mirror() {
        assert_eq!(
            IpcConnectionError::from(ErrorCode::Unknown),
            IpcConnectionError::ServerNotFound
        );
        assert_eq!(
            IpcBindingError::from(ErrorCode::Unknown),
            IpcBindingError::AlreadyBound
        );
        assert_eq!(
            IpcSendError::from(ErrorCode::Unknown),
            IpcSendError::ConnectionNotFound
        );
        assert_eq!(
            IpcReceiveError::from(ErrorCode::Unknown),
            IpcReceiveError::ConnectionNotFound
        );
        assert_eq!(DataError::from(ErrorCode::Unknown), DataError::NotFound);
    }
}
