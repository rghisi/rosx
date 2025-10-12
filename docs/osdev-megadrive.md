# Mega Drive OS - Peripheral Communication Protocol

## System Overview

A packet-based communication system for connecting a Sega Mega Drive to external peripherals via a Raspberry Pi Pico (or STM32) controller.

### Architecture

```
┌─────────────┐         ┌────────────┐         ┌─────────────┐
│  Mega Drive │◄───────►│   Pico/    │◄───────►│ Peripherals │
│   (m68k)    │  Serial │   STM32    │  SPI/I2C│  (W5500,    │
│   + Z80     │    or   │ Controller │         │   SD, RTC)  │
│             │ Parallel│  (Router)  │         │             │
└─────────────┘         └────────────┘         └─────────────┘
```

### Three-Tier System Roles

1. **Host (Address 0x00)**: Mega Drive m68k
2. **Controller (Address 0x0F)**: Pico/STM32 - acts as packet router
3. **Peripherals (Address 0x01-0xFE)**: Individual devices

## Protocol Stack

### Network Layer - Packet Format

```
┌──────────────────┬──────────┬──────────┬─────────────────┐
│  LENGTH + FLAGS  │ DST_ADDR │ SRC_ADDR │      DATA       │
│     2 bytes      │  1 byte  │  1 byte  │  0-2047 bytes   │
└──────────────────┴──────────┴──────────┴─────────────────┘
```

**Field Details:**
- **LENGTH**: 11 bits (0-2047 bytes of data)
- **FLAGS**: 5 bits (reserved for future use)
- **DST_ADDR**: 8 bits destination address
- **SRC_ADDR**: 8 bits source address
- **DATA**: Variable length payload (0-2047 bytes)

**Total Header Size**: 4 bytes
**Minimum Packet Size**: 4 bytes (header only, zero data)
**Maximum Packet Size**: 2051 bytes (4 byte header + 2047 bytes data)

**Endianness**: Little-endian (optimized for Z80 processing)

### Address Space Allocation

- `0x00`: Host (Mega Drive)
- `0x0F`: Controller itself (for management/status)
- `0x01-0x0E, 0x10-0xFE`: Peripherals

**Example peripheral addresses:**
- `0x10`: W5500 Ethernet controller
- `0x20`: SD Card
- `0x30`: Real-time clock (RTC)
- `0x31`: Temperature sensor
- `0x40-0x4F`: Reserved for future I2C devices
- `0x50-0x5F`: Reserved for future SPI devices

### Data Link Layer

**Responsibilities:**
- Stream packet bytes over physical layer
- Fragment and reassemble packets based on LENGTH field
- Maintain packet boundaries

**Operation:**
1. Read 2-byte header (LENGTH + FLAGS)
2. Extract length value from header
3. Read exactly LENGTH more bytes (DST + SRC + DATA)
4. Pass complete packet to network layer

**Design Philosophy**: Fail fast and hard. No checksums, retries, or error recovery at this layer. Corrupted length fields will cause misalignment and visible failure.

### Physical Layer - Two Implementations

#### Option 1: Serial (UART)
- **Baud Rate**: 38,400 bps initially
- **Configuration**: 8N1 (8 data bits, no parity, 1 stop bit)
- **Hardware**: Z80 bit-bangs UART on controller port
- **Bandwidth**: ~3,800 bytes/sec
- **Upgrade Path**: 115,200 bps possible with optimization

#### Option 2: 8-bit Parallel
- **Data Width**: 8 bits
- **Hardware**: Uses both controller ports (Port 1 + Port 2)
- **Bandwidth**: ~50-60 KB/sec sustained
- **Configuration**: Data bus + control signals (strobe, acknowledge)

## Z80 Role

The Z80 coprocessor acts as the communication interface between the m68k and the peripheral controller:

**Responsibilities:**
- Bit-bang UART or manage parallel interface
- Maintain ring buffers (4KB RX + 4KB TX recommended)
- Handle data link layer (packet streaming)
- Provide memory-mapped interface for m68k

**Memory Map (example):**
- `0xA00000-0xA00FFF`: RX ring buffer (4KB)
- `0xA01000-0xA01FFF`: TX ring buffer (4KB)
- `0xA01FFE`: RX head pointer
- `0xA01FFC`: RX tail pointer
- `0xA01FFA`: TX head pointer
- `0xA01FF8`: TX tail pointer

## Controller (Pico/STM32) Role

**Responsibilities:**
- Route packets between host and peripherals based on DST_ADDR
- Translate network layer packets to device-specific protocols (SPI, I2C)
- Manage device address mapping
- Handle device-specific timing and requirements

**Routing Logic:**
- If DST_ADDR = 0x00 → Forward to host (via Z80 interface)
- If DST_ADDR = 0x0F → Process locally (controller management)
- If DST_ADDR = 0x01-0xFE → Forward to appropriate peripheral

**Key Feature**: Controller is protocol-agnostic. It only inspects DST_ADDR for routing; device-specific protocols are handled at application layer.

## Application Layer (Not Standardized)

Each peripheral device defines its own protocol within the DATA field of packets. The system does not impose commands like READ or WRITE at the network layer.

**Examples:**

**W5500 Ethernet (Address 0x10):**
- Application defines register read/write commands
- SPI protocol translation happens in controller

**SD Card (Address 0x20):**
- Application defines block read/write commands
- SPI protocol and block management in controller

**RTC (Address 0x30):**
- Application defines time read/write commands
- I2C protocol translation in controller

## Design Principles

1. **Simplicity**: Minimal protocol overhead, fail-fast error handling
2. **Layering**: Clear OSI-inspired separation of concerns
3. **Extensibility**: Easy to add new devices and protocols
4. **Agnostic Transport**: Network layer doesn't know about device protocols
5. **Address-Based Routing**: Simple forwarding based on destination address

## Future Expansion Possibilities

- Add reliability features if needed (checksums, retries, sequence numbers)
- Implement higher-speed physical layer (parallel upgrade from serial)
- Add more flag bits usage (priority, QoS, fragmentation support)
- Expand address space if 256 addresses prove insufficient
- Add controller-to-controller communication (daisy chaining)

## Hardware Considerations

### RAM Expansion
- Cartridge: 512KB-1MB SRAM recommended
- Expansion Port: 1-2MB SRAM possible
- Total system: 64KB (base) + 2-3MB (expansions)

### Peripheral Controller
- **Recommended**: Raspberry Pi Pico (~$4)
  - Dual-core 133MHz
  - Hardware SPI, I2C, UART
  - PIO for custom protocols
  - 3.3V (requires level shifters for 5V Mega Drive)
- **Alternative**: STM32F103 "Blue Pill" (~$2-4)
- **Alternative**: Arduino Mega/Teensy (~$10-25)

### Level Shifting
Required for 5V Mega Drive ↔ 3.3V modern peripherals:
- TXS0108E (8-channel bidirectional)
- 74LVC245 (unidirectional)
- Voltage regulator: 5V → 3.3V

---

*Protocol designed for Rust OS development on Sega Mega Drive / Genesis with m68k (Motorola 68000) main CPU and Z80 coprocessor.*