# packages_to_extract.py
packages = {"ansible", "ansible-core", "google-auth"}

with open("requirements_all.txt", "r") as file:
    lines = file.readlines()

with open("requirements.txt", "w") as file:
    for line in lines:
        pkg = line.split("==")[0]
        if pkg in packages:
            file.write(line)
