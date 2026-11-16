# Notice

## License of this program

EDCoPilot VoiceBridge is free software. You can give it to other persons and
you can change it under the terms of the GNU General Public License, version 3
or a later version (`GPL-3.0-or-later`). The file `LICENSE` has the full text
of the license.

The program comes with no warranty. The license gives the details.

## No affiliation

This program is not affiliated with, endorsed by, or supported by these
parties:

- EDCoPilot or its author.
- Frontier Developments, the company that makes Elite Dangerous.
- VoiceAttack or its author.

The names EDCoPilot, Elite Dangerous, and VoiceAttack belong to their owners.
This program uses the names only to tell you what it works with.

The program reads the EDCoPilot files on your computer when it runs. This
repository has no data from EDCoPilot.

## The speech model

The window program can download a speech model for the local engine. The
model is not part of this repository and not part of the installer.

- Model: NVIDIA Parakeet TDT 0.6b v3, in the int8 form of the sherpa-onnx
  project (`sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8`).
- Source of the weights: NVIDIA, <https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3>.
- Source of the ONNX files: the sherpa-onnx project,
  <https://github.com/k2-fsa/sherpa-onnx>.
- License of the model: Creative Commons Attribution 4.0 International
  (`CC-BY-4.0`), <https://creativecommons.org/licenses/by/4.0/>.

Credit: the speech model "parakeet-tdt-0.6b-v3" is by NVIDIA. The sherpa-onnx
project made the ONNX version.

## Other software

This program uses Rust crates and npm packages from other authors. Each
package keeps its own license. The file `Cargo.lock` and the file
`ui/web/package-lock.json` name the packages and their versions.
